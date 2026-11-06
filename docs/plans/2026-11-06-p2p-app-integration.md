# 桌面应用接入 P2P 配对与连接

- 日期：2026-11-06（同日按 reviewer 意见修订）
- 前置：[p2p-backend-plan](2026-11-06-p2p-backend-plan.md) Part A（Host P2P 入口、配对协议、`yttt-client-core::p2p`）
- 目的：让用户在两台真机之间完成配对并连接，作为真机实测（打洞、relay、断线恢复）的入口。付费 relay 账号登录不在本步。

## 1. 现状

- Host 侧：`src/ui/workbench/remote_access.rs` 是“远程访问此电脑”页，只管 TLS。所有操作走一个 `remote_access_busy` 标志：执行中按钮全部禁用，busy 时新操作被忽略（:359、:415）。
- Client 侧：“远程服务”窗口（`src/ui/workbench/remote_connections.rs`）列出 SSH 连接与“已有 Host”（`src/ui/app/existing_host.rs`，TLS 连接码存系统凭据库）。每个远程窗口是一个独立进程 `--remote-client`，经 stdin 收 `RemoteLaunch`，由 `src/remote_host.rs::connect` 建立 `ClientCore`，再交给 `DesktopHostRuntime::from_remote`。重启后按 `SavedRemoteTarget` 恢复。

## 2. 关键决定

### 2.1 每个配对的 Host 一把设备密钥

同一时刻可能有多个远程 Client 进程（恢复多个窗口，或连着一台时再配对另一台）。如果它们共用一个 EndpointId，在同一 relay 上后连的会顶掉先连的（`SameEndpointIdConnected`），打洞消息也可能落到另一个进程。因此：

- Client 为每个配对的 Host 生成一把独立的 Ed25519 设备密钥，存系统凭据库，命名空间 `{profile}.p2p-device`，条目 ID 为 Host EndpointId 的十六进制。32 字节密钥以十六进制保存，远低于凭据库单条长度限制。
- 好处：进程之间互不干扰；不同 Host 看到的是不同的设备身份，无法互相关联；某台 Host 撤销只影响它自己。
- 代价：将来登录付费 relay 时，每把密钥分别登记，各占套餐里的一个设备名额。这在下一步（应用登录 relay 账号）里处理，比如登记时注明对应的 Host。

### 2.2 配对用的 endpoint 就是这把设备密钥

`pair()` 以 `endpoint.id()` 作为设备身份，Host 把它写进 ACL；之后的工作会话用同一把密钥签名，并且要求 iroh 层的对端与签名一致。因此流程是：解析配对码 → 生成该 Host 的新设备密钥 → 用它绑定 endpoint → `pair()`。配对成功才把密钥写进凭据库；失败或取消就丢弃。测试断言 Host ACL 里的 key 等于 `endpoint_key(device_secret)`。

### 2.3 设备密钥不经 stdin 传递

`RemoteLaunch` 里只带 Host EndpointId。远程 Client 进程用 `local_profile` 自己从凭据库读取密钥（`Zeroizing`），读不到就报错并提示重新配对，绝不静默生成新密钥。这样启动参数里不出现长期秘密。

### 2.4 配对码的复制（修订设计 §7.1）

设计原写“配对码不写入剪贴板”。但桌面对桌面必须传一段约 600 字符的字符串，而 Client 端只能粘贴，所以改为允许复制，并加缓解措施：

- 页面默认只显示二维码，文本需要点“显示”才出现。注意：屏幕上的二维码本身就含秘密，遮罩文本只是避免被肩窥，不是保护措施。这一点照实写进界面说明。
- 邀请结束（确认、拒绝、取消、过期、作废）时，如果剪贴板内容仍是这串码，就清空剪贴板。
- 配对码泄露的风险本来就有上限：码单次有效、5 分钟过期，持码者还必须让 Host 用户确认一致的 6 位码；第二个持码者出现会让邀请作废（`Compromised`）。

### 2.5 Host 页面的轮询不占 busy

配对进行中需要每秒刷新状态。这条轮询走独立的后台任务，不设置 `remote_access_busy`，不禁用按钮；用户操作优先，正在执行用户操作时到达的轮询结果直接丢弃。配对码（`SecretText`）只在 `StartPairing` 的响应里出现一次，保存在页面内存里，邀请结束就清掉。离开该页面或关闭设置窗口时自动 `CancelPairing`，不让邀请在无人看管时开着。

## 3. Host 侧：远程访问页“P2P 配对”一节

- 开关和 relay 地址输入框，可留空，留空就只走直连。修改 relay 会让 P2P 整体重启，并断开现有 P2P 会话，界面要提示这一点；配对进行中禁止修改。设置了付费账号时，显示实际生效的 relay，并注明“设备未登录 relay 账号时只能直连”。
- 状态行：状态、本机 EndpointId 前 8 位、错误。
- “配对新设备”：二维码（默认显示）、文本（点击显示，可复制）、倒计时、取消。
- 设备提交请求后：显示设备名和 6 位码（`{:06}`），说明“与设备上显示的数字一致才确认”。按钮有三个：“只允许查看”“允许完全控制”“拒绝”，不设默认按钮；确认时带上显示时的 `endpoint_id`。`Compromised`：显示警告，附一个“知道了”按钮，点击后重置邀请。
- 已配对设备列表：名称、权限、配对时间、最后在线；“撤销”带确认，确认框里可勾选“同时重置 TLS 凭据”。
- 新设备配对成功时写一条 Host 日志（设计 §6 的审计要求）。系统通知以后再做。
- 二维码：用 `qrcode` 生成矩阵并缓存，用 gpui `canvas` 一次性画出所有方块，不为每个模块建 div。按 ECC-M 估算约 v15–v17（77–85 模块），实现时实测。生成配对码时过滤链路本地地址和常见虚拟网卡（docker、桥接）的地址，既缩小码，也少暴露内网拓扑。

## 4. Client 侧：“远程服务”里的“配对的 Host”

- 新模块 `src/ui/app/paired_host.rs`，与 `existing_host.rs` 并列。添加流程：
  1. 输入名称（可空）和配对码（遮罩输入，实时校验）。
  2. 点“配对”：在自带 tokio runtime 的后台线程里，按 §2.2 生成密钥并绑定 endpoint（relay 用码里的地址，不带 token；直连开启），然后 `pair()`。设备名默认取本机主机名，按字符边界截断到 64 字节，为空时用 “yttt”。
  3. 收到 6 位码后显示“请在 Host 上确认：123 456”，可以取消。
  4. 成功后保存密钥和记录，可选“立即连接”。
- 记录存在 `client-connections/paired-hosts.json`，每条为 `{ name, host: PairedHost }`，不含秘密；上限 32 条、64 KiB；按 Host EndpointId 去重，重新配对时覆盖旧记录和旧密钥。名称为空时显示“P2P Host”加 EndpointId 前 8 位，不显示 environment_id（那是一串 UUID）。
- 列表：显示 “P2P” 标签和名称。操作：连接、编辑（名称、relay 地址提示、直连地址提示，都只是提示，不影响身份，符合 I7）、删除。删除会同时删掉本地密钥，并提示“需在 Host 上撤销才真正失效”。
- 连接：
  - 新增 `RemoteTarget::PairedHost { host: PairedHost }` 和 `SavedRemoteTarget::Paired { host: EndpointKey }`。
  - `remote_host::connect` 新增一个分支：在自建 runtime 里 `block_on(IrohEndpoint::bind(...))`；`ClientIdentity` 的 environment/profile 取自 `PairedHost`；`initialize_environment` 和 `DesktopHostRuntime::from_remote` 的参数从 `AuthToken` 放宽为 `Credential`。endpoint 随 runtime 存活，退出时调用 `close`。
  - 认证失败时不提供“替换凭据”入口，只给“重新配对”，不能借这个入口换掉 Host key（I1）。
- 恢复兼容：旧版本程序读到不认识的 `Paired` 恢复记录时会让整次恢复失败（`src/remote_restore.rs:157`）。本版本改成跳过不认识的记录，以后再加新目标就不会出这个问题。
- 路径类型：远程 Client 保留一个 `Arc<IrohConnector>` 句柄，每 2 秒读一次 `path_kind()`。变化时写进连接日志，并在远程窗口的连接状态处显示“P2P · 直连 / P2P · relay”。不能只在连接阶段上报，因为打洞通常在连上几秒后才完成。

## 5. 后端顺带修正

- `PairingSlot::claimed_label`、`decide` 检查邀请是否过期。
- Client 在等待确认时断开（取消配对或进程退出），Host 结束这次认领，邀请回到 `Open`。目前 `serve_pairing_connection` 不监听连接关闭，Host 会一直停在等待确认。

## 6. 不做

- 付费 relay 账号登录（下一步，包括每把设备密钥的登记）。
- 手机端和摄像头扫码；桌面端粘贴配对码。
- 认证成功后由 Host 下发新的地址提示（先做手动编辑）。
- 系统通知。

## 7. 任务

- D1 Client 设备密钥存储（按 Host 存，只加载、不静默新建）和 `paired-hosts.json`（去重、上限、损坏文件报错且不覆盖）。
- D2 `RemoteTarget::PairedHost`、`SavedRemoteTarget::Paired`，恢复时跳过不认识的记录；`Credential` 打通到 `initialize_environment` / `from_remote`；`connect` 的 P2P 分支和 endpoint 生命周期；路径类型观测。测试：serde 往返、恢复兼容、launch 不含密钥。
- D3 后端修正（§5）及测试。
- D4 Client 配对界面与“远程服务”列表接入，含中英文 i18n。测试：配对后 Host ACL 的 key 等于 `endpoint_key(device_secret)`；同时连两台 Host 互不干扰。
- D5 Host 页面 P2P 一节、独立轮询、配对码生命周期与剪贴板清理、离开页面自动取消、设备列表，含 i18n。
- D6 二维码：canvas 渲染与地址过滤。测试：`rqrr`（dev-dependency）解码出的内容等于原配对码。
- D7 本机验证：两个 profile 分别当 Host 和 Client，走通 配对 → 确认 → 连接 → 路径显示 → 撤销；起本地 `yttt-relay` 或 iroh-relay 测 relay 路径；clippy 和相关测试全过。然后交给用户在 macOS 与 Windows 上实测。
