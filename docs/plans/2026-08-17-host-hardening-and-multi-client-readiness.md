# yttt Host 加固与多客户端就绪实施计划

> **For agentic workers:** REQUIRED: 遵循仓库 `AGENTS.md`。使用 `@superpowers:executing-plans` 在当前会话内按顺序执行本计划；不要把计划执行委托给 subagent。每项先写能够复现缺失行为的测试，再做最小实现；只有验收条件实际通过后才能勾选。

**状态：** H0、H1、H2、H3 已完成。本计划基于 `master@b4480f8` 的代码审查结论编写。

**目标：** 先修复 Host 当前已可触发的容量缺陷，再补齐 Phase 1 遗留的正确性缺口，最后完成不需要编写任何网络代码就能提前落地的多客户端/远程前置重构。

**依据：** `docs/host-client-architecture.md`、`docs/p2p-relay-architecture.md`、`docs/plans/2026-08-12-local-host-and-remote-follow-up-checklist.md`。

**与既有清单的关系：** 本文不重复 Phase 1/2 已完成项，也不提前实现 Phase 3 的公网传输。H0 与 H1 是既有清单未覆盖的缺陷与遗留缺口；H2 与 H3 是把既有清单 P3-1、P3-3 中的抽象要求拆成可以在没有远程传输的前提下独立交付的重构，完成后 P3-1 只剩接入具体传输实现。

---

## 执行约束

- [x] H0 → H1 → H2 → H3 顺序执行；H0 未全部通过前不开始结构性重构。
- [ ] 每一项先补一个能复现缺失行为的失败测试，再做最小实现。
- [ ] 不在本计划内引入 Iroh、QUIC、Relay、设备配对或任何公网代码路径。
- [ ] 不改变 Host 的资源所有权模型：Host 仍是 terminal、SSH、项目后端和 Agent 状态的唯一权威。
- [ ] 每个新增或修改的 wire 行为都必须携带稳定资源 ID、epoch/sequence、typed failure 和显式长度上限。
- [ ] 所有队列保持硬上限与 overflow/resync 行为；不得引入无界队列。
- [ ] 只有行为测试实际通过后才能勾选；不得因为"读代码看起来对"而标记完成。

---

# H0：修复已可触发的容量缺陷

这一组是当前用户就会踩到的缺陷，且都直接影响 Phase 1 已经对外声明的能力。

## H0-1：Checkpoint 帧超过协议上限，导致高输出终端无法重新 attach

**主要区域：** `crates/yttt-host/src/terminal.rs`、`crates/yttt-host/src/lib.rs`、`crates/yttt-protocol/src/terminal.rs`、`crates/yttt-client-core/src/lib.rs`

**问题：** 原始回放环上限与协议帧上限被设为同一个数值。`RAW_REPLAY_BYTES` 为 8 MiB（`crates/yttt-host/src/terminal.rs:40`），`MAX_FRAME_BYTES` 也是 8 MiB（`crates/yttt-protocol/src/lib.rs:43`）。`HostedTerminal::checkpoint()` 无条件把整个回放环打包进 `TerminalCheckpoint.raw_replay_tail`（`crates/yttt-host/src/terminal.rs:561-564`），而 `attach_terminal` 把完整 checkpoint 放进 `Response::TerminalAttached` 走 Control 通道返回（`crates/yttt-host/src/lib.rs:1521-1524`）。只要终端累计输出达到 8 MiB，回放环即被填满并长期维持在上限，此时 checkpoint payload 加上 `SemanticViewport` 编码必然超过帧上限，`encode_message` 返回 `FrameTooLarge`（`crates/yttt-protocol/src/codec.rs:79-90`）。

`Request::RequestCheckpoint` 走同一条路径（`crates/yttt-host/src/lib.rs:1269-1277`），存在相同问题。

`AttachTerminal.after_sequence`（`crates/yttt-protocol/src/terminal.rs:254`）已在协议中定义，用途正是增量恢复，但 Host 侧从未读取该字段。

TerminalData 通道不受影响：它只发送 `checkpoint.viewport` 并丢弃 raw tail（`crates/yttt-host/src/lib.rs:933-939`），可作为正确行为的参照。

- [x] 增加回归测试：终端产生超过 `RAW_REPLAY_BYTES` 的输出后，客户端 detach 再 attach 必须成功并恢复可见内容。
- [x] 增加协议层测试：断言任何 `HostResponse` 的编码结果不超过 `MAX_FRAME_BYTES`，覆盖满回放环的 attach 与 `RequestCheckpoint`。
- [x] `Response::TerminalAttached` 与 `Response::TerminalCheckpoint` 不再携带完整 raw tail；Control 通道只返回语义 viewport 与恢复所需的 sequence 信息。
- [x] 实现 `AttachTerminal.after_sequence`：Host 按该 sequence 裁剪回放范围，客户端已持有的部分不重复回传。
- [x] 客户端已有 sequence 早于回放环起点时返回 typed `ResyncRequired`，由客户端改走全量语义快照，不得静默截断。
- [x] raw tail 的传输迁移到 TerminalData 通道并分块发送，单块受 `MAX_ATTACHMENT_OUTPUT_BYTES` 约束。
- [x] 为 checkpoint 编码后的字节数增加 diagnostics 计数与 high-water mark。

**验收：** 在一个已输出超过 8 MiB 的终端上关闭窗口并重新打开，pane 正常恢复且显示关闭期间的输出；协议测试证明任何 attach 路径的响应帧都在上限之内。

## H0-2：项目文件大小上限超过协议帧上限

**主要区域：** `crates/yttt-project-core/src/file.rs`、`crates/yttt-protocol/src/project.rs`、`crates/yttt-protocol/src/ssh.rs`、`crates/yttt-host/src/project.rs`、`crates/yttt-host/src/ssh_runtime.rs`

**问题：** `MAX_PROJECT_FILE_BYTES` 为 10 MiB（`crates/yttt-project-core/src/file.rs:10`），`README.md` 也按 10 MiB 对外声明。但 `ProjectResponse::File` 携带完整 `text: String`（`crates/yttt-protocol/src/project.rs:31-36`），SSH 路径的 `RemoteFileContent` 携带完整 `bytes: Vec<u8>`（`crates/yttt-protocol/src/ssh.rs:168-173`），两者都走 8 MiB 帧上限。介于 8 MiB 与 10 MiB 之间的合法 UTF-8 文本文件在 Host 侧读取成功，却在协议编码阶段失败，且暴露给用户的是 `FrameTooLarge` 而非"文件过大"。保存路径存在对称问题。

- [x] 增加回归测试：读取和保存一个 9 MiB 的 UTF-8 文本文件，覆盖本地项目与 SSH 项目两条路径。
- [x] 选定并记录一致的上限策略：或将两个常量对齐，或实现分块传输；不得保留两个互相矛盾的上限。
- [ ] 若选择分块，实现 `BeginRead`/`ReadChunk` 与 `BeginWrite`/`WriteChunk`/`CommitWrite`/`AbortWrite`，携带 base revision、总长度与摘要，并支持取消。
- [x] 超出策略上限时返回 typed `ResourceLimit` 并附带实际大小与限值，UI 显示可读错误。
- [x] 校准 `README.md` 与 `docs/usage.md` 中的文件大小声明，使其与实现一致。

**H0-2 策略记录：** 不对文件做分块传输。将 `MAX_PROJECT_FILE_BYTES` 从 10 MiB 对齐到 **6 MiB**，为 postcard 与帧头留余量；本地与 SSH 超限均映射 `FailureCode::ResourceLimit`。9 MiB 文件的回归测试断言返回 typed `ResourceLimit`，而不是协议层 `FrameTooLarge`。分块项因此不实施。

**验收：** 在声明上限内的任何文本文件都能完整读取与保存；超限文件返回可读的 typed 错误，不出现协议层帧错误。

---

# H1：补齐 Phase 1 遗留的正确性缺口

这一组是既有清单 Phase 1 已勾选、但实现存在具体缺口的项。它们在单客户端下不可见，在多客户端下会直接产生错误行为。

## H1-1：Interactive 租约之间缺少协商，当前为无条件抢占

**主要区域：** `crates/yttt-host/src/runtime.rs`、`crates/yttt-protocol/src/control.rs`、`crates/yttt-client-core/src/lib.rs`

**问题：** `HostRuntime::acquire_lease` 在请求 `Interactive` 且当前持有者为其他客户端时，不返回冲突，而是直接写入新租约并向旧持有者发送 `LeaseRevoked`（`crates/yttt-host/src/runtime.rs:238-263`）。`HostRuntimeError::LeaseConflict` 仅在 `validate_lease` 中产生，`acquire_lease` 从不返回它。这构成 `docs/p2p-relay-architecture.md` §10.4 明确否决的"最后请求者获胜"模型，缺少 §10.2 要求的显式 `RequestControl` / `ContinueHere` 协商路径。

既有清单 P1-1 承诺的是"observer 不抢占已有 input/resize lease"，该承诺本身成立（Observer 模式不注册租约）；缺口在 Interactive 与 Interactive 之间。

此外租约 TTL 为 15 秒（`crates/yttt-host/src/runtime.rs:26`）并依赖每次输入续期，意味着当前控制者停止输入 15 秒后，控制权可被静默接管。这是隐式时间竞争，不是显式移交。

- [x] 增加测试：两个客户端先后请求同一终端的 Interactive 租约，第二个必须收到 typed 冲突而非直接获得控制权。
- [x] `acquire_lease` 在存在其他有效持有者时返回 `FailureCode::Conflict`，附带当前持有者标识。
- [x] 新增 `RequestTerminalControl` 与 `ReleaseTerminalControl` 协商语义：请求方发起，Host 通知当前持有者，由持有者让出或超时后按策略处理。
- [x] 定义并实现无人值守时的接管策略，且该策略必须显式可配置，不得默认依赖 TTL 到期。
- [x] 租约过期与主动让出走不同事件，客户端可区分"被抢占"与"自己超时"。
- [x] 增加测试：控制者停止输入超过 TTL 后，未经协商的另一客户端不得自动获得控制权。

**H1-1 策略记录：** 默认 `UnattendedTakeoverPolicy::Never`。Interactive 租约不再按 15s TTL 失效；`AcquireTerminalLease` 遇其他持有者返回 `Conflict`。控制权仅通过 `RequestTerminalControl` + 持有者 `ReleaseTerminalControl`/`ReleaseTerminalLease`/断线移交，或显式配置 `AfterIdle` 后由请求方接管。事件区分 `TerminalLeaseReleased`、`TerminalLeaseExpired` 与既有 `TerminalLeaseRevoked`。Attach Interactive 遇冲突时降级为 Observer，避免第二客户端无法观察。

**验收：** 两个客户端之间的终端控制权只能通过显式协商转移；任何未经协商的接管都被 Host 拒绝并返回 typed 冲突。

## H1-2：文件 revision 使用本地时间戳与 64 位哈希

**主要区域：** `crates/yttt-protocol/src/project.rs`、`crates/yttt-protocol/src/ssh.rs`、`crates/yttt-host/src/project.rs`、`crates/yttt-project-core/src/file.rs`

**问题：** `ProjectFileFingerprint` 由 `modified_nanos: Option<u128>` 与 `content_hash: u64` 构成（`crates/yttt-protocol/src/project.rs:16-22`），并直接作为 `ProjectSaveMode::Check` 的 CAS 依据。`RemoteFileFingerprint` 使用 `modified_seconds: Option<u32>`，精度更低（`crates/yttt-protocol/src/ssh.rs:155-160`）。`docs/p2p-relay-architecture.md` §3.3 已明确指出该结构不应作为长期网络协议的 revision，§11.2 给出的替代是 Host 分配的单调 `revision_number` 加内容摘要。当前实现依赖 Host 本地时钟与文件系统时间精度，跨设备、跨文件系统、时钟回拨场景下不可靠。

- [x] 增加测试：时间戳相同但内容不同、以及内容相同但时间戳不同的两种情况下，CAS 行为必须正确。
- [x] 引入 Host 分配的单调 `revision_number`，与 workspace epoch 绑定，Host 重启后 epoch 递增使旧 revision 失效。
- [x] 内容摘要改用抗碰撞哈希，不使用 64 位非加密哈希作为一致性依据。
- [x] 本地与 SSH 两条路径产生同一种 revision 表示，客户端不需要区分后端。
- [x] 保存请求携带 base revision；不匹配时返回 `Conflict` 并附带当前 revision，不覆盖磁盘内容。

**H1-2 策略记录：** wire fingerprint 追加 `ContentRevision { workspace_epoch, revision_number, content_sha256 }`。CAS 比较 SHA-256 与 Host revision，不再用 mtime / 64 位哈希。`workspace_epoch` 取 Host `host_epoch`，重启后旧 revision 直接 Conflict。本地与 SSH 共用同一套 Host 分配表。

**验收：** 并发保存在任何时钟与文件系统精度下都不会静默覆盖；本地与 SSH 项目使用同一 revision 契约。

## H1-3：滚动状态下的事件积压不触发重同步

**主要区域：** `crates/yttt-host/src/lib.rs`

**问题：** Host 在多数 `broadcast` 消费点对 `RecvError::Lagged` 都有补偿：重扫 placements（`crates/yttt-host/src/lib.rs:218-221`）、重发项目变更（`:755-758`）、重发 Agent 快照（`:774-776`）、发送 `ResyncRequired`（`:980-983`）。这部分实现是可靠的。但 `crates/yttt-host/src/lib.rs:996` 处在 `display_offset != 0` 分支下对 Lagged 不做任何补偿，滚动状态下丢失的更新不会被重同步，可能导致 unseen-output 计数与实际输出偏离。

- [x] 增加测试：客户端处于滚动状态时制造事件积压，回到底部后 viewport 与 unseen-output 计数必须与 Host 权威状态一致。
- [x] 滚动状态下的 Lagged 记录待重同步标记，客户端回到底部或显式读取 viewport 时补发权威快照。
- [x] 为每个 attachment 导出 Lagged 次数与最近一次重同步原因的 diagnostics。

**H1-3 策略记录：** Control / TerminalData 的 `Lagged(n)` 按跳过条数累计 `unseen_output` 与 `lagged_events`，滚动中置 `pending_scroll_resync`。回到底部仍发权威 Snapshot；`ReadTerminalViewport` 会清标记。diagnostics schema 升到 4，新增 `attachment_resyncs`。

**验收：** 任何积压路径下，客户端可见状态最终与 Host 权威状态一致，不存在静默丢失的更新。

---

# H2：多客户端与远程的前置重构

这一组不引入任何网络代码，但完成后 Phase 3 的 P3-1 只剩接入具体传输实现。三项都是纯重构，现有约 22,000 行测试可作为回归保障，越早执行成本越低。

## H2-1：抽出与具体传输无关的 Host 服务契约

**主要区域：** `crates/yttt-host/Cargo.toml`、`crates/yttt-host/src/lib.rs`、`crates/yttt-client-core/Cargo.toml`、`crates/yttt-client-core/src/lib.rs`、新增 transport 抽象 crate

**问题：** `yttt-host` 与 `yttt-client-core` 均在 Cargo.toml 中直接依赖 `yttt-transport-local`，`serve_connection` 直接操作具体传输类型。`docs/p2p-relay-architecture.md` §7.3 要求应用协议依赖可靠 stream、connection identity 等能力抽象而非具体 SDK 类型，当前代码不满足该要求。加入第二种传输需要修改 Host 内部，而非新增一个 adapter。

- [x] 定义 transport trait：可靠双向 stream、按 `ConnectionChannel` 分流、连接身份、优雅关闭与取消。
- [x] `yttt-host` 与 `yttt-client-core` 只依赖该抽象；`yttt-transport-local` 成为其一个实现。
- [x] 增加 in-process/内存传输实现，用于确定性的 service contract 测试。
- [x] 现有集成测试在 local 与 in-process 两种传输下运行同一套契约用例。
- [x] 增加架构约束测试：`yttt-host` 与 `yttt-client-core` 的依赖图不包含任何具体传输实现。

**H2-1 策略记录：** 新增 `yttt-transport`：`TransportListener` / `TransportConnector` / `TransportStream`，并把 framing 与 handshake 从 `yttt-transport-local` 上移。Host `run` 接收一个绑定闭包而不是已绑定的 listener：单实例锁必须先于端点绑定获取，否则两个并发启动的 Host 会先抢到端点、再输掉锁，把存活 Host 的 socket 顺手 unlink 掉。`ClientCore::connect` 接收 connector。内存实现用 `tokio::io::duplex`。架构测试只检查 `[dependencies]`，允许测试依赖本地 IPC。

**验收：** 同一套 Host 契约测试在本地 IPC 与内存传输下均通过；新增传输实现不需要修改 `yttt-host` 内部。

## H2-2：清理 wire schema 中的客户端布局、绝对路径与平台耦合

**主要区域：** `crates/yttt-protocol/src/terminal.rs`、`crates/yttt-protocol/src/project.rs`、`crates/yttt-protocol/src/ssh.rs`、`crates/yttt-protocol/src/control.rs`、`crates/yttt-host/src/`、`src/ui/terminal/pane.rs`、`src/runtime/project.rs`

**问题：** 三类信息不应出现在长期 wire ABI 上，当前均已出现。

客户端布局标识进入了 Host 资源模型：`TerminalSpawnSpec` 携带 `tab_id` 与 `pane_id`（`crates/yttt-protocol/src/terminal.rs:41-42`），且 `address_fingerprint()` 把二者算入资源寻址哈希（`:54-69`），`TerminalPlacement` 同样携带（`crates/yttt-protocol/src/control.rs:231-232`）。`docs/p2p-relay-architecture.md` §8.3 明确将 `TabId`、`PaneId` 归为客户端布局。不同客户端各自生成这些 ID，会使同一逻辑终端被视为不同资源。

Host 绝对路径被返回给客户端：`ProjectResponse::Registered` 携带 `canonical_root`（`crates/yttt-protocol/src/project.rs:159-164`），`TerminalSpawnSpec.cwd` 为 Host 绝对路径字符串（`crates/yttt-protocol/src/terminal.rs:43`）。这违反 §11.1 与 §18.5。

平台细节被固化进协议：`PlatformPath` 区分 `Unix(Vec<u8>)` 与 `Windows(Vec<u16>)`（`crates/yttt-protocol/src/project.rs:4-8`），非 Host 平台的客户端需要理解另一个平台的路径表示。SSH 拓扑信息同样外泄：`connection_id` 在多个请求中作为裸字符串传递，`StoredSshCredential` 携带 `resolved_host` 与 `host_key_sha256`（`crates/yttt-protocol/src/ssh.rs:44-52`）。

- [x] 增加协议约束测试：断言客户端可见的 request/response/event 中不含 `TabId`、`PaneId`、绝对路径与 SSH 端点信息。
- [x] 终端资源寻址改用 Host 命名空间内的稳定 ID；`tab_id`/`pane_id` 退回为纯客户端 placement，不参与 `address_fingerprint`。
- [x] 引入 backend-neutral 的 `ProjectRelativePath` 表示，按 segment 编码，拒绝绝对路径、parent 与 NUL，并单独定义非 UTF-8 路径的 wire 表示。
- [x] `cwd` 与项目 root 改为相对于 workspace 的表示，Host 侧解析为实际路径。
- [x] SSH 连接对客户端表现为 opaque handle 加连接状态；不暴露 `resolved_host`、`host_key_sha256` 与远端绝对 root。
- [x] 客户端布局状态按 `ClientInstanceId` 持有与持久化，不写入 Host catalog。

**H2-2 策略记录：** `TerminalSpawnSpec` / `TerminalPlacement` 去掉 `tab_id`/`pane_id`；`address_fingerprint` 只哈希 `(project_id, cwd, execution)`。wire 路径改用 `ProjectRelativePath` / `HostPath` / `PathSegment`（含非 UTF-8 `Bytes`）。`Registered` 不再返回 `canonical_root`，桌面保留本地 root。本地 spawn `cwd` 相对已注册项目根；SSH `RegisterSsh.root` 与 cwd 按相对 `/` 的 segment 解释。`StoredSshCredential` 只保留 id、effective_user 与 optional key identity。客户端 tab/pane 仍由桌面 placement store 持有。

**验收：** 协议约束测试通过；两个客户端可以各自使用不同的 tab/pane 布局观察同一终端资源，互不影响对方的布局与焦点。

## H2-3：改用可演进的 wire 编码并放宽同版本约束

**主要区域：** `crates/yttt-protocol/src/codec.rs`、`crates/yttt-protocol/src/handshake.rs`、`crates/yttt-transport/src/auth.rs`

**问题：** 协议使用 postcard 编码（`crates/yttt-protocol/src/codec.rs:71`）。postcard 为非自描述格式，枚举变体按声明位置编号、结构体按字段顺序读取，因此在 `Request` 中间插入变体会改变其后所有变体的编号，结构体新增或删除字段会造成错位。`ClientHello` 上的 `#[serde(default)]` 标注（`crates/yttt-protocol/src/handshake.rs:59-65`）在此格式下不提供向后兼容能力，因为编码中不存在可供跳过的字段标签。

当前之所以不出错，是握手阶段严格比对 `build.resource_compatibility`，不一致即拒绝（`crates/yttt-transport/src/auth.rs`）。这是正确的 fail-closed 行为，但同时意味着所有客户端必须与 Host 同版本发布。独立发布周期的客户端（尤其是需要应用商店审核的移动端）无法满足该约束。`docs/p2p-relay-architecture.md` §8.2 要求控制消息使用可演进的 schema。

- [x] 增加编码兼容性测试：旧版本编码的消息可被新版本解码，新增字段被安全忽略，未知枚举变体被安全拒绝而非错位解析。
- [x] 增加测试确认当前 postcard 编码在字段增删场景下的实际行为，作为迁移前的基线记录。
- [x] 控制消息改用带字段标签的可演进编码；保留 `MAX_FRAME_BYTES`、CRC 校验与帧头结构。
- [x] 终端输出与文件块等大体积 payload 保持紧凑二进制 framing，不进入带标签的结构化编码。
- [x] 握手区分 `protocol_range` 兼容与 `build_fingerprint` 相同两个概念：协议版本在支持区间内即可通信，仅在明确不兼容时拒绝。
- [x] 定义并记录 wire 演进规则：变体只追加不重排、字段只追加不复用编号、废弃字段保留占位。
- [x] 移除或修正 `#[serde(default)]` 等在当前编码下不生效的兼容性标注。

**H2-3 策略记录：** 结构化控制/握手/lifecycle/desktop-shell payload 改用 CBOR map（`ciborium`）。帧头仍为 v1。资源/lifecycle/desktop-shell 协议版本升到 2。`Vec<u8>` 大块字段用 `serde_bytes` 编成 CBOR 字节串。postcard 基线测试保留在 `tests/postcard_baseline.rs`。握手只按 `protocol_range` 协商，不再因 `resource_compatibility` / `build_fingerprint` 不同而 fail-closed。演进规则见 `docs/wire-evolution.md`。`ClientHello` 上的 `serde(default)` 在 CBOR 下生效。由此 `tests/process_roles.rs` 中断言"旧 build 的 Host 必须被替换"的用例已改写为断言新语义：协议版本仍能协商时，旧 build 的 Host 被复用而不是被拆掉；`is_resource_incompatibility` 仍把 `VersionMismatch` 视为需要替换。

**验收：** 相邻两个协议版本的客户端与 Host 可以互通；不兼容版本收到 typed 拒绝而不是错位解码。

---

# H3：资源与安全加固

这一组不阻塞 H2，可并行执行，但应在开启任何远程访问之前完成。

## H3-1：约束回放环的常驻内存

**主要区域：** `crates/yttt-host/src/terminal.rs`、`crates/yttt-host/src/diagnostics.rs`

**问题：** `RawReplayRing` 使用 `VecDeque<u8>`，填满后长期维持在 `RAW_REPLAY_BYTES` 即 8 MiB（`crates/yttt-host/src/terminal.rs:267-306`）。该成本按终端数量线性增长，16 个活跃终端即 128 MiB，是 Host 内存的主要变量项。`docs/p2p-relay-architecture.md` §14.2 要求按 terminal、workspace、SSH connection 与 attachment 分项导出资源指标。

- [x] 增加测试：多个高输出终端并存时，Host 常驻内存不超过按终端数计算的预算上限。
- [x] 按订阅状态分级：无客户端订阅的终端继续 drain 与解析，但缩减回放环保留量。
- [x] 回放环容量可按 profile 配置，并与 H0-1 的分块传输策略保持一致。
- [x] 按终端导出回放环占用、丢弃字节数与 high-water mark。

**H3-1 策略记录：** 无订阅终端回放环缩到 `UNSUBSCRIBED_REPLAY_BYTES`（256 KiB）；有订阅仍为 8 MiB。容量由 `HostRuntimeConfig.replay_budget` 配置。16 个无订阅高输出终端上界为 4 MiB，由 `ReplayBudget::host_limit` 测试锁定。diagnostics schema 升到 5，导出 `replay_bytes` / `replay_capacity` / `replay_dropped_bytes` / `replay_high_water`。

**验收：** 16 个高输出终端场景下的 Host 常驻内存有明确上界，并由基准脚本记录。

## H3-2：收敛可由客户端指定的命令面

**主要区域：** `crates/yttt-protocol/src/project.rs`、`crates/yttt-protocol/src/ssh.rs`、`crates/yttt-host/src/project.rs`、`crates/yttt-host/src/ssh_runtime.rs`

**问题：** `ProjectRequest::Git` 接受完整的自由参数列表（`crates/yttt-protocol/src/project.rs:150-154`），`RemoteCommandRequest` 接受任意 `program` 与 `args`（`crates/yttt-protocol/src/ssh.rs:248-253`）。Host 侧没有命令或参数白名单。在本地单信任域下这与用户自身权限等价，但一旦开启远程访问，任何获得连接的客户端都可以借助 `git -c` 一类参数执行任意程序，且无法区分 `git.read` 与 `git.mutate` 权限。`docs/p2p-relay-architecture.md` §9.3 要求 capability 至少区分这两者。

- [x] 增加测试：通过 `git -c` 或等价参数注入外部程序的请求必须被 Host 拒绝。
- [x] Git 请求改为受限操作集合，每个操作有明确语义与参数结构，而非透传参数数组。
- [x] 受限操作集合按读与写分类，为后续 capability 检查预留挂载点。
- [x] `RemoteCommand` 同样收敛为受限操作，或明确标注为高权限操作并在远程场景默认关闭。
- [x] 记录被拒绝请求的审计条目，不记录文件内容与终端明文。

**H3-2 策略记录：** `ProjectRequest::Git` 改为 `ProjectGitOperation` 受限集合。Host 用 `current_dir` 代替 `-C`，`git_argv_is_safe` 拒绝 `-c`/`-C`/`--exec-path`/`--git-dir`/`--work-tree`。`RemoteCommand` 收敛为 `Git { operation }` 或 `Privileged`；后者默认 `PermissionDenied`。拒绝写入审计日志，不记录文件内容或终端明文。

**验收：** 客户端无法通过 Git 或远程命令接口执行受限操作集合之外的程序；读写操作可在协议层区分。

## H3-3：为设备身份与 capability 预留协议位置

**主要区域：** `crates/yttt-protocol/src/control.rs`、`crates/yttt-protocol/src/handshake.rs`、`crates/yttt-host/src/lib.rs`

**问题：** 当前所有客户端共享同一个对称 bearer token（`crates/yttt-transport-local/src/auth.rs:19-35`），Host 无法区分客户端来源、无法单独吊销某台设备、无法做差异化授权。在 `crates/yttt-host/src` 与 `crates/yttt-protocol/src` 中检索 `capability`、`device_id`、`acl` 均无匹配。完整的设备身份与 ACL 属于既有清单 P3-3，本项只做协议预留，避免 P3-3 时再次变更已稳定的 envelope。

- [x] `ClientRequest` envelope 增加 `actor_device_id` 与可选 `lease_epoch` 字段，本地路径填入稳定的本机设备标识。
- [x] 定义 capability 枚举与 typed `PermissionDenied` 失败，本地 profile 默认授予全部 capability。
- [x] Host 在每个 mutating 请求入口预留 capability 检查点，当前实现为恒真但路径必须存在。
- [x] 为 mutating 请求记录 actor、resource、request ID 与结果的审计条目。
- [x] 增加测试：capability 检查点被绕过的请求路径会导致测试失败。

**H3-3 策略记录：** `ClientRequest` 追加 `actor_device_id` 与 `lease_epoch`（`serde(default)`）。`Capability` 枚举覆盖 terminal/project/git/ssh/privileged/credential。`Request::required_capability()` 为穷尽匹配；本地 profile 检查点恒真。ClientCore 用 `client_instance_id` 填 `actor_device_id`。mutating 结果写入 `HostAuditLog`。

**验收：** P3-3 实现真实设备身份与 ACL 时，只需替换检查点实现与身份来源，不需要变更 wire envelope。

## H3-4：定义多客户端下的凭据挑战归属

**主要区域：** `crates/yttt-protocol/src/ssh.rs`、`crates/yttt-host/src/ssh_runtime.rs`、`src/ui/workbench/ssh_connections.rs`

**问题：** `ServerEvent::CredentialChallenge` 向所有订阅客户端广播（`crates/yttt-protocol/src/control.rs:195`）。多客户端同时在线时，谁有权应答、非发起方是否应看到挑战、重复应答如何处理，均未定义。`CredentialAnswer` 携带明文密码（`crates/yttt-protocol/src/ssh.rs:113-119`），在本地 UDS 下可接受，在远程场景下必须重新评估。

- [x] 增加测试：两个客户端在线时发起 SSH 连接，只有发起方收到并可应答凭据挑战。
- [x] 凭据挑战定向发送给发起该连接请求的客户端，不做全局广播。
- [x] 发起方断线时挑战超时并使连接尝试失败，不转交其他客户端。
- [x] 非发起方只收到不含敏感信息的连接状态变更。
- [x] 明确记录凭据类消息不得进入可重放的 request journal。

**H3-4 策略记录：** SSH 事件改为 `Broadcast` / `Unicast`。Host-key 挑战只 unicast 给 `connect` 发起方；状态变更仍广播。非发起方应答返回 `PermissionDenied`。发起方断线 `abandon_challenges` 拒绝未决挑战，不转交。`SshConnect` / `CredentialAnswer` 继续不可 journal。

**验收：** 多客户端场景下凭据挑战只对发起方可见可答；其他客户端只观察到连接状态。

**复审修复记录：** 独立复审后修正四处缺陷。`PathSegment` 改为 `#[serde(try_from)]`，使反序列化与 `utf8`/`bytes` 构造函数走同一套校验，堵住通过 wire 直接投递 `..` 段、进而让 Git 在项目根之外执行的路径；`register_attachment` 对已存在的 attachment 只置 `attached`，不再清空 `pending_raw_after`、`display_offset` 与 `pending_scroll_resync`；`git_argv_is_safe` 只检查 `--` 之前的参数，避免名为 `-cache.txt` 的未跟踪文件被当成 `-c`；`request_creates_resource` 补上 `RegisterSsh`，使 draining 期间不再接受新的 SSH 项目注册。

---

## 与既有清单的映射

| 本计划项 | 既有清单对应项 | 关系 |
|---|---|---|
| H0-1、H0-2 | 无 | 新发现的缺陷，既有清单未覆盖 |
| H1-1 | P1-1 | P1-1 只覆盖 observer 不抢占；补齐 Interactive 之间的协商 |
| H1-2 | P3-4 中的 CAS save | 提前修正 revision 表示，避免远程阶段返工 |
| H1-3 | P1-2 | 补齐既有 resync 机制在滚动状态下的空隙 |
| H2-1 | P3-1 第 1 项 | 完成后 P3-1 只剩接入具体传输 |
| H2-2 | P3-1 第 3 项 | 提前完成路径与布局的 wire 清理 |
| H2-3 | P3-1 第 2 项 | 提前完成 schema 版本化与升级兼容 |
| H3-1 | P1-6 资源预算 | 补充按终端的内存上界 |
| H3-2、H3-3 | P3-3 | 提前收敛命令面并预留 capability 挂载点 |
| H3-4 | P3-3、P3-4 | 定义多客户端凭据语义 |

---

## 不在本计划范围内

以下不属于本计划，需按既有清单在对应阶段执行：

- Iroh/QUIC 接入、NAT 穿透、Relay 与端到端加密（P3-2）。
- 真实设备配对流程、密钥轮换与撤销（P3-3）。
- 移动端客户端与推送唤醒（Phase 4）。
- Host-owned 文档草稿、writer handoff 与协作（Phase 5）。
- Host 重启后从磁盘恢复 PTY、独立 headless binary、terminal cell 级压缩（非默认后续项）。

---

## 最终完成条件

- [x] H0 全部通过：高输出终端可重新 attach，声明上限内的文件可正常读写。
- [x] H1 全部通过：终端控制权只能显式转移，revision 模型与后端无关，积压路径最终一致。
- [x] H2 全部通过：Host 契约与具体传输解耦，wire schema 不含客户端布局与绝对路径，相邻协议版本可互通。
- [x] H3 全部通过：Host 内存有明确上界，命令面收敛，capability 检查点与审计路径就位。
- [ ] `cargo fmt --check`、`cargo test --workspace` 与严格 clippy 全部通过。
- [x] 更新 `docs/host-client-architecture.md` 与 `docs/p2p-relay-architecture.md` 中受本计划影响的状态描述。
