# Host 与远程链路韧性计划

> **For agentic workers:** REQUIRED: 遵循仓库 `AGENTS.md`。在当前会话内按顺序执行本计划，不要把计划执行委托给 subagent。每项先写能复现缺失行为的测试，再做最小实现；只有验收条件实际通过后才能勾选。

**状态：** A–D 已实现。遗留：C2 行为测试待 SSH server 夹具；D1 首次读取与写入仍同步；D2 期限仍从入队计；D3 待手工验证。

**依据：** 2026-11-04 的空闲断连分析（`e46c64c` 已修复断线丢租约）与同日对 Host、SSH 隧道、TLS、Host 托管 SSH 项目的代码审查；`docs/host-client-architecture.md`。

**目标：** 远程链路在网络切换、休眠、慢链路下能在有界时间内自动恢复，恢复期间不制造重连风暴；控制状态（lease、退出、连接）不因队列溢出丢失；远程配置访问不阻塞 UI；每一次断连都能事后定位原因。

---

## 执行约束

- [ ] A → B → C → D 顺序执行；每阶段单独提交，阶段内每项先有失败的测试。
- [ ] 不改变 Host 的资源所有权与认证模型；本机 IPC、TLS、SSH 入口的检查保持不变。
- [ ] 所有队列保持硬上限；新增的重连、重试都有上限与退避，永久错误不重试。
- [ ] 协议只做追加：新增字段带 `#[serde(default)]`，旧端忽略后行为不变；改动 wire 形状时重写 `wire-shapes-v22.json` 并说明兼容性。
- [ ] 每阶段结束时同步 `docs/host-client-architecture.md` 中对应章节与 §13 验证清单。
- [ ] 无法用自动化覆盖的行为（真实网络切换、App Nap）写明手工验证步骤，不以“没有报错”代替验证。

---

# A：链路活性与重连

**问题：** SSH 隧道重连要依次经过“在旧会话上开通道（僵死时每次 15 s，试两次）→ 重建 SSH → 远端 `ensure`”，却被 ClientCore 统一的 30 s 连接期限截断，旧会话要 45–60 s 才被 keepalive 判死，期间重连永远到不了重建这一步；心跳只认 Pong，慢链路上 Pong 排在已发出的数据之后而误判超时；重连后所有终端在上报可见性之前就重新订阅，Host 视为全部可见并推送全量快照，响应里又附一份快照，进一步拖慢心跳，形成风暴；断连原因不落盘。

**主要区域：** `crates/yttt-client-core/src/lib.rs`、`crates/yttt-transport/src`（connector trait）、`src/remote_host.rs`、`crates/yttt-ssh/src/transport.rs`、`crates/yttt-host/src/lib.rs`、`crates/yttt-protocol/src/control.rs`。

## A1：任何入站数据都证明连接存活

- [x] 测试：Host 持续输出终端数据但 Pong 被延迟超过心跳期限时，Client 不断线；Host 完全静默超过期限时仍断线。
- [x] `connected_session` 记录最后一次入站消息时间；心跳超时以“自上次入站起”计算，Pong 只是入站的一种。
- [x] connector 可声明链路特性（`TransportConnector::link_profile()` 之类，默认本机）：远程（SSH、TLS）心跳期限放宽到 45 s，本机保持 15 s。

**验收：** 慢链路上持续有数据时不再因 Pong 排队而重连；真断线的检测时间本机不变、远程有界。

## A2：SSH 隧道重连在有界时间内完成

- [x] 测试（以可注入的假 SSH 服务驱动 `RemoteConnector`）：旧会话开通道超时一次后，立即重建 SSH，不再在同一僵死会话上重试；远端服务正常时重连不运行 `ensure`；隧道报“endpoint 不存在/拒绝连接”时才运行 `ensure` 并重试一次。
- [x] `RemoteConnector::connect`：开通道失败即视为会话不可用，在 gate 内先检查 SSH epoch 是否已被他人重建，否则重建；`ensure` 只在会话正常但隧道目标不可达时执行。
- [x] streamlocal 开通道期限独立于 exec 的 15 s，缩短到 8 s。
- [x] connector 声明自己的连接期限（远程 90 s，本机仍 30 s），ClientCore 用它代替固定的 `CONNECT_TIMEOUT`。

**验收：** 网络切换后，SSH 隧道在“检测断线 + 一次 SSH 建连”内恢复，不再无限循环；`ensure` 次数在正常重连中为 0。

## A3：重连后不制造快照风暴

- [x] 测试：重连时可见性上报先于重新订阅、重新订阅带 `subscribe_only`（recovery_tests）；`subscribe_only` 得到 `Applied` 且流仍以 snapshot 开始（host_roundtrip）。不可见终端只收元数据沿用既有的 `hidden_terminal_streams_metadata_only_until_revealed`。
- [x] `connected_session` 在重新订阅之前先发送 `SetTerminalVisibility`。
- [x] `RequestCheckpoint` 追加 `#[serde(default)] subscribe_only: bool`；为 `true` 时 Host 只建立订阅、返回 `Applied`。旧 Host 忽略该字段返回 checkpoint，Client 两种响应都接受。

**验收：** 重连后不可见终端不再推送全量快照、重新订阅不再重复附带 checkpoint。**未做：** 按帧计数断言首轮下行量的端到端测试。

## A4：断连原因可追溯

- [x] Client：每次离开 `Ready` 记录原因、时间与持续时长（diagnostics 保留最近 N 条）；Desktop 把连接状态变化带时间戳写入日志。
- [x] Host：会话连接建立/结束带时间戳与 client id、结束原因（读错误、写超时、溢出、被替换、停止）写入 `host.log`；单向输入被拒的日志带时间戳并限流。

**验收：** 一次人为断网可以从 Client 与 Host 两端日志读出断开时刻、原因与恢复耗时。

---

# B：控制状态不丢失

**问题：** Host 把每帧画面更新与 lease/控制事件放在同一个 256 容量广播里，溢出时控制事件被丢；桌面每个 pane 订阅全局广播、pane 事件队列只有 16 格且阻塞发送，UI 慢时 `TerminalExit`、lease 事件、`HostLost` 会丢，溢出处理只刷新元数据。

**主要区域：** `crates/yttt-host/src/runtime.rs`、`crates/yttt-host/src/lib.rs`（`forward_state_events`）、`src/host_runtime.rs`、`src/ui/terminal/pane.rs`。

## B1：Host 控制事件独立通道并可对账

- [x] 测试：大量终端输出使画面事件溢出时，lease 撤销/授予仍送达 Client；人为让控制通道溢出后，Client 收到按当前状态补发的 lease 事件。
- [x] runtime 拆出控制事件广播（lease、control、exit），画面 `Update` 不再进入它；`forward_state_events` 不再为画面更新锁 attachment 表。
- [x] 控制通道 lag 时，按本连接每个 attachment 的当前 mode 与 runtime lease 对比，补发 `TerminalControlGranted` / `TerminalLeaseRevoked`。

## B2：桌面 pane 以状态对账代替丢事件

- [x] 测试：pane 事件桥 lag 后，pane 仍得到最终的退出与 lease 状态。
- [x] pane 事件桥 lag 时发送 `Resync`；pane 收到后按 catalog placement 的 `owner` 与 mirror 元数据重新判定 lease 与退出，并在连接状态为 `HostLost` 时显示断开。

**验收：** 控制状态在任意事件丢失后都能收敛到 Host 的当前状态。

---

# C：Host 读循环与 SSH 运行时

**问题：** Host 处理恢复请求或普通队列满时不读 socket，心跳请求得不到回复；SSH 连接 actor 在命令循环里内联等待，开终端通道没有超时，会卡住同一连接上的 SFTP、exec、隧道；Host 托管的 SSH 项目断线后不会自动重连，SSH 状态事件溢出即丢。

**主要区域：** `crates/yttt-host/src/lib.rs`、`crates/yttt-ssh/src/transport.rs`、`crates/yttt-host/src/ssh_runtime.rs`。

## C1：Host 读循环与请求处理解耦

- [x] 测试：普通请求排满且有慢请求执行时，Ping 仍在期限内得到回复。
- [x] Host 的 session 读取改为独立任务；Ping 由读任务直接回复；恢复请求移出读循环。

## C2：SSH 连接 actor 不内联等待

- [ ] 测试：一个开通道请求挂起时，同一连接的 SFTP 与隧道请求仍能完成。**未完成：** 仓库没有进程内 SSH
  server 夹具，只有需要外部 endpoint 的 `host_ssh_product_smoke`；该测试待夹具就绪后补。
- [x] 循环内所有等待（`channel_open_*`、远端转发、断开）带期限。**偏离：** russh `Handle` 不可克隆且需
  `&mut` 轮询会话结束，开通道无法移出循环；改为有界内联等待，卡住的通道最多推迟其他命令一个期限。

## C3：Host 托管 SSH 项目自动重连

- [x] 测试：重连决策（只重连建立过的连接、同时只有一个重连循环、循环期间再次断开不丢失、显式断开后停止）与退避上限有单元测试；永久错误停止由 `TransportError::is_permanent` 判定。端到端重连同样待 SSH server 夹具。
- [x] SSH 状态事件 lag 时按当前连接表补发状态。
- [x] 重连成功后执行 §7.0 的 pane 协调，恢复 SSH pane（复用既有的 `Connected` → `ensure_all`）。

**验收：** 心跳在 Host 负载下不再误判；SSH 项目网络恢复后无需手动重连。

---

# D：远程配置与系统节流

**问题：** 远程环境下配置存储是同步网络往返（最长 125 s），部分调用在 UI 线程；普通请求 10 s 期限不适合慢链路上的大响应；UI 进程被 App Nap 降权，放大误判。

**主要区域：** `src/host_storage.rs`、`src/config/storage.rs` 及其 UI 调用点、`crates/yttt-client-core/src/lib.rs`、desktop 启动代码。

## D1：UI 线程不做远程配置往返

- [x] 审计 `config::storage` 的调用点：约 74 处，集中在 `ui/theme/zed/installed.rs`（20）、`config/project_settings.rs`（17）、`config/layout_loader.rs`（14），其余分布在 keybindings、theme、settings、bars、layout editor 与设置页；它们经同步的 `ConfigStorage` 接口到达 `HostStorage`，调用线程即等待网络。
- [x] 测试：远程配置读取在首次之后不等待网络，断开后仍立即返回缓存；其他 Client 的写入经通知刷新缓存；Host 磁盘上的修改由周期刷新发现；本地写入使用最新 revision（`tests/remote_config_cache.rs`）。
- [x] `HostStorage`（仅远程）缓存配置读取并在后台刷新；Host 对经它的配置变更广播 `ServerEvent::ConfigChanged`，Client 收到后与重连后刷新缓存。
- [ ] **未完成：** 同一路径的首次读取、目录探测（`is_dir`/`read_dir`）与写入仍同步等待网络；彻底消除需要把 `ConfigStorage` 调用方改为异步，留作后续。

## D2：请求期限按请求大小与链路区分

- [x] 远程链路上 `Workspace` 与 `Project` 请求使用 120 s 期限（由 connector 的 `LinkProfile` 判定）。
- [ ] **未完成：** 期限仍从入队开始计算；改为从发出开始需要调整过期请求在发送前被丢弃的语义，留作后续。

## D3：阻止 App Nap

- [x] 进程启动时以 `NSProcessInfo beginActivityWithOptions`（`UserInitiatedAllowingIdleSystemSleep`）声明活动并持有到进程结束。**偏离：** 未按“有终端附加或远程连接时”启停——UI 进程总有 Host 会话，全程持有更简单且等价。
- [ ] 手工验证（未执行）：用本分支构建的 App 放到后台闲置几分钟后，`ps -o pid,pri,comm -p <UI pid>` 的优先级不再降到 4，`logs/client.log` 中没有断开记录。

**验收：** 远程慢链路或断线时，已读取过的配置不再冻结 UI；后台闲置不再触发断连（待 D3 手工验证）。

---

# E：TLS 直连

**问题：** Host 读远程连接没有期限，也没有 TCP keepalive，对端休眠或断网后连接永不关闭，占住远程客户端名额与终端 lease；监听循环任何一次 accept 出错就退出且不自动恢复；导出的连接码使用监听地址，`0.0.0.0` 或回环地址对其他机器不可达；客户端把 DNS、TCP 与 TLS 合在 5 s 内；任一连接失败让所有新连接暂停 100 ms。

**主要区域：** `crates/yttt-host/src/lib.rs`、`crates/yttt-host/src/remote_access.rs`、`crates/yttt-transport-tls/src/lib.rs`、`src/ui/workbench/remote_access.rs`。

## E1：Host 关闭静默的远程连接

- [x] 测试：网络入口的 Session 连接超过空闲期限没有任何入站消息时，读任务以期限原因结束；本机连接不受影响（`session_idle_tests`）。
- [x] TLS 与 SSH 入口的 Session 读取带空闲期限（60 s，客户端每 5 s 发心跳）；连接关闭后进入既有的 lease 宽限期。
- [x] TLS 两端开启 TCP keepalive 作为兜底（30 s 后每 10 s 探测，3 次）。

## E2：监听循环不因瞬时错误退出

- [ ] 测试：accept 出错后监听继续接受连接。**未完成：** `TlsListener` 没有可注入 accept 错误的接缝，进程内制造
  `EMFILE` 会影响同进程的其他测试；改由下一项的端到端测试覆盖监听恢复。
- [x] accept 出错时退避重试（上限 1 s），不退出循环。
- [x] 测试：Host 启动时监听地址被占用，地址释放后自动恢复监听（`remote_access_rebind.rs`，去掉重绑定后该测试失败）。
- [x] 监听任务意外结束或启动时绑定失败，管理器按退避（1 s 起，上限 30 s）自动重新绑定；用户操作取消待定的重试。

## E3：建连期限与节流

- [x] 客户端 TCP 建连（含解析）15 s、TLS 握手 10 s 分开计时；多个解析地址错开 250 ms 并行尝试。
- [x] Host 的 TLS 加认证握手期限由 5 s 放宽到 10 s。
- [x] 测试：握手失败只延迟同一来源 IP 的后续连接，不影响其他来源；每个 IP 只有一个连接等待；表大小有上限。
- [x] 握手失败的节流改为按来源 IP 指数退避；已认证连接正常结束不再触发节流。**偏离：** 被节流的连接延迟到
  节流结束再握手而不是直接拒绝（同 IP 的合法 Client，例如 NAT 之后或刚用错凭据的同一台机器，仍能连上），
  每个 IP 同时只有一个在等待；上限 5 s，低于 Client 的 TLS 握手期限。

## E4：可达的导出地址

- [x] 测试：监听地址为未指定地址时，导出地址替换为本机出站地址；具体地址原样导出。
- [x] 导出连接码时，未指定地址换成本机出站地址；复制提示显示连接码中的地址，回环地址另提示只能本机连接。
  出站地址取路由表选择的本机地址；有 VPN 时可能是 VPN 地址，导入端仍可编辑地址。

**未纳入 E：** 桌面持有的 Host（`DesktopOwned`）随桌面退出而停止，远程客户端的终端随之结束——退出确认已
提示会停止 Host，但未单独提示远程客户端；远程 Client 每次启动使用新的 client ID，旧会话的 lease 需等宽限期。

**验收：** 远程客户端断网后 Host 在约 1 分钟内释放其名额与 lease；监听不会因瞬时错误永久停止；弱网下建连不再被 5 s 截断；导出的连接码可直接在其他机器使用。
