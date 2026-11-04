# Host 与远程链路韧性计划

> **For agentic workers:** REQUIRED: 遵循仓库 `AGENTS.md`。在当前会话内按顺序执行本计划，不要把计划执行委托给 subagent。每项先写能复现缺失行为的测试，再做最小实现；只有验收条件实际通过后才能勾选。

**状态：** A、B、C 完成（C2 的行为测试待 SSH server 夹具）；D 进行中。

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

- [x] 测试：Client 重连时只有可见终端收到全量快照，不可见终端只收元数据；重新订阅的响应不再携带 viewport。
- [x] `connected_session` 在重新订阅之前先发送 `SetTerminalVisibility`。
- [x] `RequestCheckpoint` 追加 `#[serde(default)] subscribe_only: bool`；为 `true` 时 Host 只建立订阅、返回 `Applied`。旧 Host 忽略该字段返回 checkpoint，Client 两种响应都接受。

**验收：** 32 个终端、只有 4 个可见时，重连后的首轮下行数据量按可见数而非总数增长（测试中以帧计数断言）。

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

- [ ] 审计 `config::storage` 的全部调用点，列出 UI 线程上的读写。
- [ ] 测试：远程配置读取在连接慢或断开时不阻塞调用线程（缓存命中立即返回，未命中返回明确错误）。
- [ ] `HostStorage` 在绑定时预取配置并按变更事件更新缓存，读取走缓存；写入异步提交，UI 线程不等待。

## D2：请求期限按请求大小与链路区分

- [ ] 远程链路上 `Workspace` 打开等大响应使用较长期限；期限从请求真正发出开始计算，不含排队。

## D3：阻止 App Nap

- [ ] 有终端附加或远程连接时，以 `NSProcessInfo beginActivityWithOptions` 声明用户活动；全部断开时结束。手工验证：后台闲置后 `ps` 中 UI 进程优先级不再降到 4。

**验收：** 远程慢链路或断线时 UI 不冻结；后台闲置不再触发断连。
