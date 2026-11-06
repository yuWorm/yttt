# 桌面应用接入 P2P 配对与连接

- 日期：2026-11-06
- 前置：[p2p-backend-plan](2026-11-06-p2p-backend-plan.md) Part A（Host P2P 入口、配对协议、`yttt-client-core::p2p`）
- 目的：让用户在两台真机之间完成配对并连接，作为真机实测（打洞、relay、断线恢复）的入口。付费 relay 账号登录不在本步。

## 1. 现状

- Host 侧：`src/ui/workbench/remote_access.rs` 是“远程访问此电脑”页，只管 TLS（开关、监听地址、复制连接码、重置凭据、已连接客户端）。
- Client 侧：“远程服务”窗口（`src/ui/workbench/remote_connections.rs`）列出 SSH 连接与“已有 Host”（`src/ui/app/existing_host.rs`，TLS 连接码存系统凭据库）。连接时启动独立进程 `--remote-client`，经 stdin 传 `RemoteLaunch { target: RemoteTarget, saved_target }`，由 `src/remote_host.rs::connect` 建立 `ClientCore`。重启后按 `SavedRemoteTarget` 恢复打开的远程窗口。

## 2. Host 侧：远程访问页增加“P2P 配对”一节

位于 TLS 一节之下，互不影响：

- 开关（`SetP2p`）与 relay 地址输入框（可留空，留空则只能直连；填 `http(s)://` 地址）。状态行显示状态、本机 EndpointId 前 8 位、错误。
- “配对新设备”按钮（`StartPairing`）：显示配对码文本（遮罩，可显示/复制）与二维码，剩余有效时间倒计时，“取消”（`CancelPairing`）。
- 设备提交请求后（`PairingState::AwaitingConfirmation`）：显示设备名与 6 位码，提示“与设备上显示的数字一致才确认”；按钮“允许完全控制”“只允许查看”“拒绝”（`ConfirmPairing{preset}` / `CancelPairing`）。`Compromised` 时显示警告：配对码可能泄露，邀请已作废。
- 已配对设备列表（`ListDevices`）：名称、权限、配对时间、最后在线；“撤销”带确认，确认框里可勾选“同时重置 TLS 凭据”（`RevokeDevice`）。
- 配对进行中每秒刷新一次状态（现在页面只在操作后刷新）；离开页面或配对结束后停止。
- 二维码：用 `qrcode` crate 生成矩阵，直接用 gpui 的 `div` 网格画黑白方块（码长 ≤1 KiB，字母数字模式约 57×57 模块），不引入图片管线。

## 3. Client 侧：“远程服务”增加“配对的 Host”

- 本机设备密钥：每个应用 profile 一把 Ed25519 密钥，首次配对时生成，存系统凭据库（命名空间 `{profile}.p2p-device`），不落明文文件。同一把密钥对所有 Host 使用；某个 Host 撤销它不影响其他 Host。
- 添加流程（新模块 `src/ui/app/paired_host.rs`，与 `existing_host.rs` 并列）：
  1. 输入名称（可空，默认用 Host 名）与配对码（遮罩输入，`PairingCode::decode` 实时校验）。
  2. “配对”：后台线程建临时 `IrohEndpoint`（relay 取配对码里的地址，不带 token；直连开启），调用 `yttt_client_core::p2p::pair`。设备名默认本机主机名。
  3. 收到 6 位码后显示“请在 Host 上确认：123 456”，可取消。
  4. 成功后保存记录，可选“立即连接”。
- 记录：`client-connections/paired-hosts.json`，元素为 `{ name, host: PairedHost }`。`PairedHost` 只含公钥、地址提示和 ID，不含秘密，所以可以放普通文件；上限 32 条、64 KiB。
- 列表：显示“P2P”标签与 Host 名；操作：连接、重命名、删除（只删本地记录，提示需在 Host 上撤销才真正失效）。
- 连接：新增 `RemoteTarget::PairedHost { host: PairedHost, device_secret: String }`（秘密在 `zeroize_secrets` 中清零），`SavedRemoteTarget::Paired { host: EndpointKey }` 用于重启恢复。`remote_host::connect` 新分支：在远程 Client 进程里建 `IrohEndpoint`（直连开启，relay 用记录里的地址），`PairedHost::connector` + `PairedHost::credential`，复用 `initialize_environment`（参数从 `AuthToken` 放宽为 `Credential`）。
- 连接状态界面显示路径类型（直连 / relay），便于实测时判断打洞是否成功：`remote_connect.rs` 目前写死 “TLS” 的地方改为按目标显示 “P2P · 直连 / P2P · relay”。

## 4. 不做

- 付费 relay 账号登录（下一步）。
- 手机端与摄像头扫码；桌面端用粘贴配对码。
- 已配对记录跨设备同步。

## 5. 任务

- D1 `RemoteTarget::PairedHost`、`SavedRemoteTarget::Paired`、`initialize_environment` 接受 `Credential`、`remote_host::connect` 的 P2P 分支；路径类型上报。测试：serde 往返、秘密清零、恢复键稳定。
- D2 Client 设备密钥（凭据库）与 `paired-hosts.json` 读写。测试：大小上限、损坏文件报错不覆盖、记录增删改。
- D3 Client 配对界面与“远程服务”列表接入（含 i18n 中英文）。
- D4 Host 远程访问页 P2P 一节、配对确认、设备列表、轮询（含 i18n）。
- D5 二维码渲染。测试：生成的矩阵能被 `qrcode` 自身的解码校验或与已知向量比较。
- D6 本机验证：用 `run` 方式启动两个 profile（一个当 Host，一个当 Client），截图走通 配对 → 确认 → 连接 → 撤销；`cargo clippy`、相关 crate 测试全过。然后交给用户做 macOS↔Windows 实测。
