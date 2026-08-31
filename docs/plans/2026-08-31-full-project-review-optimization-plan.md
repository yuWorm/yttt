# yttt 全项目审阅优化实施计划

> **For agentic workers:** REQUIRED: 遵循仓库 `AGENTS.md`。使用 `@superpowers:executing-plans` 在当前会话内按顺序执行本计划；不要把计划执行委托给 subagent。每项先建立可复现的行为测试、约束测试或基线数据，再做最小实现；只有验收条件实际通过后才能勾选。

**状态：** G0 仓库内实现已完成；等待新 `Required validation` check 在 GitHub 首次成功运行后配置 branch protection。R1 按顺序在该门禁生效后开始。本计划基于 `master@6d3bcbf` 与 `docs/reviews/2026-08-31-full-project-review.md` 编写。

**目标：** 先建立不可绕过的验证门禁，再闭合恢复路径、Agent 代次、SSH credential、资源上限和边界正确性；性能优化必须由 profile 驱动；最后补齐发布分发的完整性与真实性。

**依据：** `docs/reviews/2026-08-31-full-project-review.md`、`docs/host-client-architecture.md`、`docs/plans/2026-08-12-local-host-and-remote-follow-up-checklist.md`、`docs/plans/2026-08-17-host-hardening-and-multi-client-readiness.md`。

**与既有计划的关系：** 本计划不重做已经完成的 Host 资源所有权、多客户端、transport abstraction、wire evolution 与 terminal performance 基础设施。既有计划中的架构决定继续有效，尤其是 Host 权威、客户端有界 mirror、SSH endpoint 对客户端保持 opaque、所有 queue 有硬上限、所有 mutating request 有幂等策略。

---

## 实施前冻结的设计决定

以下决定用于消除审阅建议与现有架构之间的冲突，执行时不得临时选择第二套约定。

1. **验证门禁先行。** 第一批只建立快速、稳定、不可绕过的 validation；恢复路径修复紧随其后，不等待全部发布基础设施完成。
2. **Release 不跨 workflow 使用 `needs`。** 抽出 `workflow_call` 可复用 validation workflow，由 build 与 release 在精确 commit/tag 上分别调用。
3. **握手错误保留结构化分类。** 不把 `HandshakeError` 提前转成字符串；区分 transient、Host replacement required、permanent 三种处置。
4. **terminal-data worker 与 supervisor 共用同一分类。** worker 不得对 permanent failure 无限重试，也不得自行触发 Host replacement。
5. **Agent generation 只有一个 attempt 真相源。** pane spawn 环境、AgentRuntime reducer、Host process event 与 hook ingestion 必须使用同一个 `(instance_id, generation)`。
6. **SSH credential binding 由 Host 强制执行。** 不重新把 `resolved_host`、port、host fingerprint 加回 `StoredSshCredential` wire 类型；credential secret 与完整 binding 作为同一个 Host-owned keyring envelope 保存。客户端继续只持有 opaque credential id。
7. **旧的无 binding credential fail closed。** 不从当前可被篡改的 connection 配置推导可信 fingerprint；旧条目需要重新认证并保存。
8. **semantic scroll 区分 requested 与 acknowledged offset。** 现有 `SemanticViewport.rows` 只对应 Host 已确认 viewport；没有额外历史行缓存时，不通过替换 offset 伪造乐观内容滚动。
9. **request journal 保留重连窗口。** control connection 退出时不立即删除；使用 TTL、LRU、client 数量与全局字节上限回收。
10. **remote root `/` 不是自动判定的安全缺陷。** 默认值与交互需要收紧，但显式选择 `/` 的合法管理场景保留。
11. **性能结论先实测。** review 中 UI、文件搜索、PTY resize 和锁争用结论主要来自静态审阅；未取得基线前不做跨模块缓存框架或线程模型重构。
12. **发布 checksum 与签名分层。** SHA-256 负责损坏/错配检测；签名或签名 manifest 负责发布者真实性，二者不可互相替代。

---

## 全局执行约束

- [ ] 严格按 G0 → R1 → A2 → S3 → C4 → M5 → U6 → D7 推进；同一阶段中明确标注独立的项可以并行准备，但不得绕过前置验收。
- [ ] 每个 repository-changing task 在独立 `./.worktrees` worktree 中执行；所有编辑、命令和验证都在该 worktree 内完成。
- [ ] 每个缺陷先增加能在旧实现上失败的行为测试；纯 workflow 变更必须执行对应命令并验证 failure path。
- [ ] 修复源状态机或契约，不通过吞错、延长 sleep、扩大无界队列、增加 silent fallback 掩盖症状。
- [ ] clean cutover：迁移所有 caller，删除旧分类、旧字段、旧分支与兼容 alias；不保留两套 credential 或 generation 语义。
- [ ] 所有新增 queue/cache/journal/index 都必须有硬上限、overflow/eviction 行为和 diagnostics。
- [ ] 所有 timeout 都返回 typed operation/context；超时后关闭或废弃不能安全复用的 stream/process handle。
- [ ] UI render/prepaint/paint 路径不做无条件模型 mutation、全量 clone 或全量重建；缓存使用源 revision，不在每帧做深比较替代重算。
- [ ] 性能改动必须保存 before/after 原始数据，场景、构建模式、机器、窗口、运行次数保持一致。
- [ ] 不跳过、ignore 或降低断言来处理 FSEvents 环境失败；实现实际降级路径并在可用 macOS runner 上验证原生 watcher。
- [ ] 不在日志、diagnostics、测试 fixture 或错误中记录 password、passphrase、terminal input、clipboard、env value 或文件正文。
- [ ] 每个阶段完成时更新本计划状态、相关架构文档和 CHANGELOG；只有实际验证通过后勾选。

---

# G0：建立验证与发布门禁

## G0-1：固定工具链并抽出可复用 validation workflow

**主要区域：** `rust-toolchain.toml`、`.github/workflows/build.yml`、新增 reusable workflow、workspace test 配置

- [x] 记录当前通过 workspace build/test 的 Rust 版本并用 `rust-toolchain.toml` 固定；后续升级必须独立 PR 验证。
- [x] 新增 `workflow_call` validation workflow，在调用方 checkout 的精确 commit 上运行。
- [x] validation 运行 `cargo fmt --all --check`。
- [x] validation 运行 `cargo clippy --workspace --all-targets --no-deps --locked -- -D warnings`；若 vendor 仍是 workspace member，则使用明确 package allowlist，不修改 vendor 源码压制告警。
- [x] Linux 运行 workspace 全量 headless tests；macOS 运行 GPUI/UI、FSEvents 与 bundle tests；Windows 保留 library、IPC、process lifecycle 与 installer smoke。
- [x] validation 纳入 `python3 scripts/test_release_tools.py` 与 `python3 scripts/test_terminal_perf_summary.py`。
- [x] 保留现有 local IPC security、Linux Host SSH smoke、三平台 package smoke 与 bundled Host lifecycle 验证；不得用新 validation 替换这些深度 smoke。
- [x] 将三个 watcher 相关测试放在 FSEvents 可用的 macOS runner 上执行；环境异常必须显式报告，不静默 skip。
- [ ] branch protection 要求完整 validation 结果，而不是只要求 matrix 中单一 job。
  **Blocked：** 当前 `master` 尚无该 check context；在 workflow 合入并首次成功运行前设置 required context 会锁死分支。首次成功运行后通过 GitHub branch protection 要求 `Validation / Required validation`。

**验收：** 未格式化、任一自有 clippy warning、workspace 行为回归、release tool 回归或任一 required platform smoke 均阻止合并；vendor warning 不进入自有 `-D warnings` 门禁。

**G0-1 实施记录（2026-08-31）：**

- Rust 固定为 `1.96.0`，使用 minimal profile，并安装 clippy/rustfmt。
- `.github/workflows/validation.yml` 提供 Linux 自有 workspace strict clippy/全量测试、macOS UI/FSEvents/bundle、Windows IPC/library/lifecycle 三组验证，以及稳定的 `Required validation` 汇总状态。
- Cargo 实际把四个 vendor package 视为 workspace member；strict clippy 与 Linux tests 因此使用 15 个自有 package allowlist，未修改 vendor 源码。
- 本地 fmt、actionlint、release tools、terminal performance summary、CLI 回归与自有 crate strict clippy 通过。当前 workstation 的 FSEvents 静默失效仍稳定触发三个既有 watcher 测试失败；CI 不 skip，并在 macOS job 真实执行。一次并行全套件中的 macOS bundle timeout 隔离重跑通过。

## G0-2：让 release 在打包前验证精确 tag

**主要区域：** `.github/workflows/release.yml`、reusable validation workflow、`scripts/release_metadata.py`、`scripts/prepare_release.py`

- [x] release checkout 精确 tag 后调用同一个 validation workflow；macOS/Windows/Linux packaging jobs 全部 `needs` 该 validation job。
- [x] `workflow_dispatch` 输入的 tag 与 push tag 使用同一验证路径，不允许手动发布绕过。
- [x] `release_metadata.py` 在目标版本 CHANGELOG 段缺失、为空、仍使用旧版本内容或 `Unreleased` 未正确归档时非零退出。
- [x] `scripts/test_release_tools.py` 覆盖当前 `CHANGELOG.md` 中已存在目标版本标题时的 failure/repair 行为。
- [x] README release checklist 明确要求运行 `prepare_release.py`、检查 diff、跑 validation、再创建 tag。
- [x] 删除 `gh release upload --clobber`；同 tag 或同名资产已存在时发布失败，禁止可变 release。
- [x] `validate_asset` 严格匹配 `release_metadata.ASSET_FILENAMES`，测试 mock 使用同一规范名。
- [x] 将 `/dist` 加入 `.gitignore`，避免本地产物误提交。

**验收：** 在未通过精确 tag validation、CHANGELOG 错位、规范资产缺失或 release 已存在时，不创建、不更新、不覆盖任何 GitHub Release 资产或 `update.json`。

**G0-2 实施记录（2026-08-31）：**

- release 的 metadata job 先确认输入是指向 checkout commit 的真实 tag，并验证 package version 与 CHANGELOG；随后 reusable validation 在同一 tag ref 上运行，三个 packaging job 均依赖它。
- `release_metadata.py --validate-changelog-only` 要求 `Unreleased` 为空且目标版本是其后的第一个唯一 release section。当前未归档的 `0.2.0` 状态会明确失败，无法发布旧 notes。
- `prepare_release.py --repair-current` 可以把当前版本已有标题前的 Unreleased 内容合并进去，保留旧 notes 且不产生重复标题；无显式 flag 时继续 fail closed。
- updater 只接受当前 platform/version 的规范资产文件名；release workflow 不再更新或 `--clobber` 已发布资产。

---

# R1：闭合 Client/Host 恢复路径

## R1-1：统一握手错误分类

**主要区域：** `crates/yttt-transport/src/auth.rs`、`crates/yttt-client-core/src/lib.rs`、`src/host_runtime.rs`

- [ ] 增加纯函数测试，覆盖每个 `HandshakeError` 与 `RejectReason`，确保新增枚举变体必须显式分类。
- [ ] 引入结构化 disposition：`Transient`、`HostReplacementRequired`、`Permanent`；error message 只用于展示，不参与控制流。
- [ ] `Wire`、`Timeout` 分类为 `Transient`。
- [ ] `Rejected(StaleHostEpoch | AlreadyConnected | HostShuttingDown)` 分类为 `Transient`。
- [ ] `Rejected(BuildMismatch | VersionMismatch)` 分类为 `HostReplacementRequired`，交给 Desktop Host launcher，而不是无限 reconnect 或直接永久失联。
- [ ] `Rejected(ProfileMismatch | AuthenticationFailed | InvalidMessage)`、`UnexpectedMessage`、`IdentityMismatch`、`HandshakeError::AuthenticationFailed` 分类为 `Permanent`。
- [ ] transient 保留 mirror 并有界指数退避；replacement 暂停 lane worker 并触发单一 launcher transaction；permanent 进入带 typed reason 的用户可见状态并停止重试。
- [ ] 删除 `map_err(|error| ConnectFailure::Fatal(error.to_string()))` 及任何重复分类。

**验收：** Wire/Timeout 连续失败后 Host 恢复时客户端自动回到 Ready；profile/auth/invalid message 只尝试一次；build/version mismatch 只触发一个 Host replacement transaction。

## R1-2：让 terminal-data worker 遵守同一处置

**主要区域：** `crates/yttt-client-core/src/lib.rs`

- [ ] 为 terminal-data worker 增加 deterministic connector 测试，统计连接尝试次数并使用可控时钟。
- [ ] transient failure 才进入退避；成功后清零 attempt。
- [ ] replacement/permanent failure 退出 worker、关闭对应 sender/stream，让订阅者收到终止或全局状态变化。
- [ ] worker 不直接启动 Host，不与 supervisor 形成两个恢复控制器。
- [ ] shutdown、desired=false 与 fatal/replacement 同时发生时保证 task 退出且不泄漏 sender。

**验收：** 永久认证错误下，每个 terminal 最多一次 data-lane 连接尝试；没有存活的重连 task、额外文件描述符或 CPU wakeup。

## R1-3：Host 恢复失败可观察且 CLI 有超时

**主要区域：** `src/host_runtime.rs`、`src/host_launcher.rs`、`src/main.rs`

- [ ] 将 `let Ok(process) = launch_or_attach()` 改为显式处理 success/error；记录 typed recovery stage、attempt、last error。
- [ ] transient launch error 有界退避；不可恢复错误进入用户可见 `RecoveryFailed`/等价状态，不无限显示模糊的 Reconnecting。
- [ ] recovery success 原子替换 managed process 并清除最后错误；同时到达的多个 reconnect event 只能有一个 launch transaction。
- [ ] `HostControlClient::request()` 增加 10 秒总超时；超时包含 operation/request id，并废弃当前 stream。
- [ ] `wait_for_host_artifacts_to_clear` 同时确认 ready/pid 消失、profile lock 已释放、endpoint 已不可连接。
- [ ] existing-host 启动等待区分“正常启动未发布 ready”与“持锁 Host 长期不可达”，错误信息给出 stage 与 elapsed。

**验收：** 模拟 spawn permission error、磁盘错误、半开 control stream、Host drain 与慢启动；所有 CLI/UI 操作在有界时间内成功或返回可定位错误，不 hang、不静默重试。

---

# A2：修复 Agent attempt 代次与有界投递

## A2-1：统一 pane generation 与 reducer generation

**主要区域：** `src/runtime/agent_manager.rs`、`src/ui/terminal/pane.rs`、`crates/yttt-agent-runtime/src/lib.rs`

- [ ] 增加回归测试：第一次 spawn 失败后重试、新 generation hook 早于 process-started event、旧 generation 失败迟到。
- [ ] 在发起 Host spawn 之前调用显式 `begin_process_attempt(instance_id, generation)` 或等价 API，使 reducer 先进入相同 generation 的 Starting。
- [ ] pane generation 保持单调且禁止 `0`；发生 wrap 时终止旧 instance 并分配新 instance，不复用已使用 generation。
- [ ] spawn environment、Host request、process-started/process-exited 与 hook envelope 都读取同一 attempt generation。
- [ ] `process_start_failed` 使用传入 generation；只有 `(instance_id, generation)` 与当前 attempt 完全匹配才允许清理。
- [ ] 匹配失败时清理 `launches_by_address`、`addresses_by_instance`、AgentRuntime record 与不再可恢复的 retained snapshot；保留 resume override 的语义必须由测试锁定。
- [ ] stale failure、stale exit、stale hook 均不得改变当前 attempt。

**验收：** spawn 失败后立即重试会创建真实新 attempt；generation 2 hook 在任何 process event 排序下都被正确接收；generation 1 的迟到事件不影响 generation 2。

## A2-2：给 OMP hook 投递设置语义化上限

**主要区域：** `crates/yttt-agent-providers/src/yttt-agent-extension.ts`、Agent provider tests

- [ ] 用故障 Host 测试持续产生 hook，证明当前实现队列增长，并将其固化为回归测试。
- [ ] 队列设置硬上限；上限值、当前深度、high-water、coalesced 与 dropped 数量可观察。
- [ ] 中间 working/progress 状态按 instance/session 合并，只保留最新有意义状态。
- [ ] completed/failed/cancelled 等终态优先于可丢弃中间状态，不因队列满被静默抛弃。
- [ ] 恢复连接后按每 instance 的有效顺序投递，不回放已被新快照覆盖的旧状态。
- [ ] `user_prompt`、Waiting message、OpenCode entries 等所有 hook 文本统一走现有 bounded text/clip 契约。

**验收：** Host 长期不可达和 busy agent 场景下 extension 常驻内存有固定上限；恢复后最终 AgentSnapshot 与真实终态一致。

## A2-3：修复 Agent 项目边界与阻塞扫描

**主要区域：** `src/runtime/agent_sessions.rs`、`crates/yttt-agent-providers/src/command.rs`、`src/runtime/notification.rs`

- [ ] `belongs_to_project` 使用 canonical path component 边界，`/proj` 不匹配 `/project`。
- [ ] OpenCode CLI 扫描移到 background task，设置 timeout，并在超时时 kill/reap child。
- [ ] `session_idle` 不再映射为 Completed；复用现有 Working/Idle 语义或先冻结产品决定再增加状态。
- [ ] Claude/OpenCode index 的解析条目数有硬上限。
- [ ] notification 以 authoritative `ExitReason` 与 exit code 共同判断，不把无 code 的合法 Completed 误报失败。

**验收：** 挂起 CLI 不阻塞 UI；相似路径项目不会串 session；turn 间 idle 不闪烁 completed。

---

# S3：完成 SSH credential 与 SFTP 一致性

## S3-1：将完整 credential binding 与 secret 一起保存在 Host

**主要区域：** `crates/yttt-ssh/src/credential.rs`、`crates/yttt-ssh/src/transport.rs`、`crates/yttt-host/src/ssh_runtime.rs`

- [ ] 定义版本化 keyring envelope，包含 password secret、effective user、resolved host、port、verified host-key SHA-256 与 private-key identity。
- [ ] envelope 的 Debug/error 始终 redact secret；临时解码 buffer 使用 `Zeroizing`/`SensitiveBytes` 并在 drop 时清零。
- [ ] 新保存流程在 host-key 验证完成后构造 binding，并将 secret 与 binding 原子写入同一 credential entry。
- [ ] stored-password authentication 只接收 opaque credential id；Host 读取 envelope 后强制比较 user/host/port/fingerprint/identity。
- [ ] 完整 binding 校验通过前不得调用 `authenticate_password`；任一字段不匹配返回 `CredentialBindingMismatch`。
- [ ] 删除 `resolved_host.is_empty()` fail-open 分支与 Host 侧构造空 binding 的转换。
- [ ] 不给 `StoredSshCredential` wire 类型重新增加 endpoint/fingerprint 字段，保持既有 opaque SSH 边界。

**验收：** 篡改 host、port、user、fingerprint 或 identity 的任一测试都在发送密码前失败；协议约束测试继续证明客户端 wire 不暴露 Host SSH endpoint binding。

## S3-2：迁移旧 credential 并纠正客户端语义

**主要区域：** `src/ui/workbench/ssh_connections.rs`、SSH config migration、README/usage docs

- [ ] keyring 中旧的裸 password 被识别为 `LegacyUnbound`，不得自动认证。
- [ ] UI 提示用户重新输入一次 password；成功验证 host key 后保存新版 envelope。
- [ ] 不从当前 `ssh-connections.toml` 或用户刚批准的新 fingerprint 静默补齐旧 credential。
- [ ] 客户端 config 中的 `CredentialBinding` 明确降为 UX/cache 信息或删除；安全判定只信任 Host-owned envelope。
- [ ] 编辑 connection endpoint 时可以保留 opaque credential id，但下一次 Host binding mismatch 必须转为重新认证流程，不自动发送旧 secret。
- [ ] README 将“remembered password 绑定 verified endpoint”的实现位置与迁移行为写清楚。

**验收：** 升级后的旧用户不会把未绑定密码自动发送到任何 endpoint；重新保存后后续连接正常，配置文件仍不包含 password/passphrase。

## S3-3：序列化 SFTP mutation 并补超时与清理

**主要区域：** `crates/yttt-ssh/src/transport.rs`、`crates/yttt-ssh/src/sftp.rs`

- [ ] 为同一 connection/session 的 save/delete/rename/mkdir 增加单一 mutation actor 或 mutex；多步 save transaction 全程持有。
- [ ] 先保持 read/list 是否并发的现状；只有底层库与压力测试证明安全时才允许与 mutation 并发。
- [ ] `client::connect` 增加显式 connect timeout；timeout 后取消并释放 connection slot。
- [ ] save 首次冲突判断优先使用 metadata/revision；只有有冲突嫌疑时读取全文，避免固定两次完整下行。
- [ ] save 前清理超过明确 TTL 的 `.yttt-*.tmp/.bak`；成功后的 backup 删除有重试、日志与上限。
- [ ] symlink N+1 先建立同次 expand cache；没有数据证明前不做全局持久 metadata cache。

**验收：** 并发 save/read/delete 压力测试不交错 mutation transaction、不留下半成品；不可达 SSH host 在固定超时内释放 runtime slot。

## S3-4：校准 remote root 产品策略

**主要区域：** SSH connection form、remote path tests、README

- [ ] 新 connection 默认 root 改为用户 home 或显式待选择状态，不默认 `/`。
- [ ] 用户选择 `/` 时显示“允许访问该账号可见的整个远程文件系统”并要求明确确认。
- [ ] `is_within` 保持严格 component 语义；非 `/` root 继续拒绝 sibling prefix 与 symlink escape。
- [ ] `/` 的合法场景保留，不把它错误描述为 sandbox bypass。

**验收：** 默认流程不会无提示选择整个远程文件系统；显式选择 `/` 与受限 root 都有对应 confinement 测试和文档。

---

# C4：修复边界正确性与资源有界性

## C4-1：统一 semantic scroll 的显示与交互状态

**主要区域：** `crates/yttt-terminal/src/view.rs`、`crates/yttt-terminal/src/render/content.rs`、`src/ui/terminal/pane.rs`

- [ ] 增加 GPUI 行为测试：wheel/scrollbar 请求、RPC 在途、Host viewport ack、连续滚动与 hit testing。
- [ ] 分离 `requested_scroll_offset` 与 `acknowledged_scroll_offset`，并记录请求 generation/sequence。
- [ ] semantic rows、selection、cursor、hyperlink hit test 与内容 paint 始终使用同一个 acknowledged viewport。
- [ ] scrollbar 可以表示 pending target，但不得让 thumb/content/hit test 分别使用三个 offset。
- [ ] 新 Host viewport 到达时只 ack 对应或更新的请求；迟到 viewport 不覆盖更新的 requested target。
- [ ] 连续 wheel 事件合并为最新 target，避免无界 ScrollTerminal RPC。
- [ ] 若后续需要乐观内容滚动，先增加有界历史行 cache 与 miss 行为；本项不通过移动旧 rows 冒充新 viewport。

**验收：** 任意 RPC 延迟下，用户看到的内容、scrollbar、selection 与 hit testing 保持自洽；Host ack 后内容和 thumb 在同一 frame 收敛到目标位置。

## C4-2：保证 checkpoint 最终收敛并限制 request journal

**主要区域：** `crates/yttt-client-core/src/lib.rs`、`crates/yttt-host/src/lib.rs`、Host diagnostics

- [ ] checkpoint 调度改为每 session 至多一个 pending；Full 时合并，不静默丢失最新 resync 需求。
- [ ] control request 完成或队列释放后重新调度仍 pending 的 checkpoint。
- [ ] 为 sequence gap、ResyncRequired、queue Full、coalesced 与最终 checkpoint success 增加 diagnostics。
- [ ] request journal 使用 TTL + LRU + client 数量上限 + 全局字节上限；单 client 的 256 条/4 MiB 上限继续保留。
- [ ] journal eviction 不删除当前连接或重连窗口内仍可能 replay 的 entry；达到全局上限时按最旧 inactive client 淘汰。
- [ ] 增加 1,000 个 client instance churn、断线重连 replay 与全局内存上限测试。

**验收：** 任意 checkpoint burst 最终恢复 authoritative mirror；Independent Host 经大量 Desktop 重启后 journal RSS 有固定上界，重连幂等 replay 仍成立。

## C4-3：文件 watcher 降级与 dirty-close 队列

**主要区域：** `src/ui/workbench/settings.rs`、project watcher、`src/ui/workbench/document_lifecycle.rs`

- [ ] watcher 构造成功但不投递事件的测试使用 fake backend/fake clock 重现。
- [ ] 原生 watcher 继续作为低延迟通道；增加低频 metadata/revision reconciliation。
- [ ] reconciliation 发现磁盘变化但没有对应 native event 时，将该 root 切换为 `PollWatcher` 并显示 degraded 状态。
- [ ] 不在用户项目目录写 `.yttt-watch-probe`；keybindings 可直接周期 stat，项目目录使用已有 tree/document revision 做 reconciliation。
- [ ] watcher 恢复策略、poll 间隔与扫描上限可测试，不形成新的全项目每秒 walk。
- [ ] dirty file close 从单一 `pending_dirty_close` 改为有界 FIFO；一个对话框完成后自动处理下一项。
- [ ] 相同 document 的重复 close 合并；Cancel 只取消当前请求，不丢弃后续独立请求。

**验收：** native watcher 静默失效后外部编辑仍被检测并显示降级；快速关闭多个 dirty 文件时每个 close 意图都被确认、保存、丢弃或明确取消。

## C4-4：移除 panic/递归与边界错误

**主要区域：** `crates/yttt-host/src/terminal.rs`、`crates/yttt-project-core/src/tree.rs`、`crates/yttt-terminal/src/view.rs`

- [ ] Host PTY reader/writer thread spawn 返回 `Result`，映射 typed resource failure，不 panic 整个 Host。
- [ ] `copy_entry` 在递归前使用 `symlink_metadata`；不跟随 directory symlink，或显式复制 symlink，并限制深度/entry 数量。
- [ ] terminal `ColorRequest` 查询失败时返回协议允许的默认/空应答，不让调用方永久等待。
- [ ] remote environment name 限制为 `[A-Za-z_][A-Za-z0-9_]*`；非法 name 返回 typed validation error。
- [ ] SSH host-key metadata 文件权限与其他 profile config 策略一致；公钥 fingerprint 不按 secret 记录，但避免不必要的宽权限。

**验收：** thread exhaustion、symlink 环、颜色查询失败和非法 env name 都返回可观察错误；Host 不 panic、复制不无限递归、调用方不永久等待。

---

# M5：建立 UI 与终端性能基线

## M5-1：为 Workbench render 热点增加观测

**主要区域：** `src/ui/workbench/render.rs`、`src/ui/workbench/project_files.rs`、`src/ui/workbench/surface.rs`、`src/ui/project_tree/view.rs`

- [ ] 记录 Workbench render 总时长及 project tree snapshot/sync、work-area reconcile/clone、tab snapshot/sort、palette build/filter、font detection 的分段时长。
- [ ] 记录每段 invocation、cache hit/miss、重建条目数、clone bytes/entry count 与 p50/p95/p99。
- [ ] instrumentation 受现有 perf feature 控制，关闭时不分配、不锁、不格式化字符串。
- [ ] 增加结构性测试：只有 terminal damage 变化的一帧，project tree、work area、tab metadata 与 command registry 的 rebuild count 均为零。
- [ ] 覆盖 10k/100k files、50 tabs、深展开 project tree、1k commands、持续 terminal damage、onboarding font step。

**验收：** 每个报告中的 UI 热点都有可重复基线，可区分“调用次数过多”和“单次调用过慢”；没有数据前不进入 U6 的结构重构。

## M5-2：扩展文件搜索与 terminal 基线

**主要区域：** `src/runtime/file_search.rs`、`src/runtime/project.rs`、`crates/yttt-terminal/src/perf.rs`、`scripts/run-terminal-perf.sh`

- [ ] 文件搜索记录 enumeration、normalization、query scoring、cancel latency、candidate count、peak memory 与 first-result latency。
- [ ] 基线覆盖 git repository、非 git repository、ignored files、100k files、快速连续输入与取消。
- [ ] terminal 继续运行 direct/host 的 full、damage、scroll、burst、interactive，每 cohort 至少 5 runs，并保存 raw data。
- [ ] 保持现有门槛：Host throughput 至少为 Direct 90%；Host echo-to-paint p95 增量不超过 3 ms；Host input-to-paint p95 不超过 Direct 2 倍；Host idle CPU <0.5%；combined one-pane 增量内存 ≤15 MiB。
- [ ] 增加 parser lock wait、semantic queue age、prepaint/paint allocation 与 overlay/damage row count 的 before 数据。

**验收：** summarizer 严格通过；所有性能优化都能绑定到一个已保存的高占比指标，而不是仅凭源码位置决定优先级。

---

# U6：按基线消除 UI 与 terminal 热点

## U6-1：Project tree 使用 revision 驱动同步

**主要区域：** `src/ui/workbench/project_files.rs`、`src/ui/project_tree/view.rs`、project editor runtime

- [ ] project tree model 暴露单调 revision；snapshot cache key 至少包含 model revision、icon-theme revision、show-hidden 与 locale/text revision。
- [ ] key 未变化时 `ensure_project_tree_view` 不构建 snapshot、不调用 `sync_with_icon_theme`、不 `cx.notify()`。
- [ ] `rows_by_id` 与 render rows 使用 `Arc`/不可变共享快照，删除每帧两次 HashMap 深 clone。
- [ ] 先用 M5 数据确认 `tree()` 是否已经窗口化；只有确认所有可见行都构建且占比显著时才迁移虚拟列表。
- [ ] edit/context-menu 所需 row 数据引用同一 snapshot，不复制第二份菜单 map。

**验收：** terminal-only frame 的 project-tree rebuild/sync 为零；大树 model 真正变化时只更新一次，交互与 context menu 行映射保持正确。

## U6-2：缓存 Work area 与 tab snapshot

**主要区域：** `src/ui/workbench/work_area.rs`、`src/ui/workbench/surface.rs`、workspace/document state

- [ ] `reconcile_work_area` 仅在 terminal/file IDs、order-customized 或 layout revision 变化时运行。
- [ ] 默认 order 未变化时不替换根 `WorkAreaNode`；保存 revision 与稳定 node identity。
- [ ] `selected_work_area_snapshot` 使用 `Arc<WorkAreaNode>` 或等价不可变 snapshot，删除每帧深 clone。
- [ ] tab metadata 在 document dirty/missing、terminal title/status、active item 或 order revision 变化时增量更新。
- [ ] 排序先建立 `item_id -> rank` map，删除 comparator/key 中反复 `order.iter().position()` 的近似 O(n²) 查找。

**验收：** 50 tabs 持续 terminal 输出场景下，未变化 tab/work-area 的 snapshot build 与 deep clone 为零；tab 顺序、dirty/missing 标记和焦点行为保持一致。

## U6-3：拆分 palette 静态数据与 query 数据

**主要区域：** `src/ui/workbench/render.rs`、palette/picker components、CommandRegistry

- [ ] command descriptor/keybinding display cache key 使用 registry revision、keymap revision 与 locale。
- [ ] palette 打开后，query keystroke 只做 filtering/ranking，不重新遍历 registry 或解析 keybinding string。
- [ ] picker 使用虚拟列表，只构建 viewport 与 overscan rows。
- [ ] 删除无条件 `items.to_vec()`；overlay 与 picker 共享不可变 items snapshot。
- [ ] onboarding font detection 变成一次性状态 transition；字体枚举与宽度探测不会在 Pending 状态的每个 render 重跑。

**验收：** 1k command 场景快速输入时静态 palette build 只发生一次；每次 query 的 UI-thread p95 由 M5 基线定义并受回归测试约束。

## U6-4：建立有界、可增量的文件索引

**主要区域：** `src/runtime/file_search.rs`、`src/runtime/project.rs`、project watcher

- [ ] 首次 enumeration 在 background 执行，UI 不同步等待全量 walk。
- [ ] 每个 project 缓存一次 normalized/lowercase candidate；query 不重复 lowercase 全列表。
- [ ] 增加 input debounce、generation cancellation 与 top-K 截断；旧 generation 在有界时间停止 CPU 工作。
- [ ] 优先使用 git tracked + untracked/non-ignored enumeration；非 git 项目保留 `ignore::WalkBuilder`。
- [ ] 文件数量、单路径长度、索引字节数与返回结果数都有硬上限；达到上限显示 truncated 状态。
- [ ] watcher event 增量更新内存索引；native watcher degraded 时 reconciliation 仍能最终收敛。
- [ ] 只有 M5 证明重启后的首次索引仍不可接受时才增加磁盘持久索引；本项默认不引入数据库。

**验收：** 100k files 下首次 enumeration 不阻塞 UI；连续 query 不重复 walk；取消的 query 在固定时间内停止；索引内存有固定上限。

## U6-5：优化 terminal 已证实的热点

**主要区域：** `crates/yttt-terminal/src/render/cache.rs`、`crates/yttt-terminal-core/src/semantic.rs`、`crates/yttt-terminal/src/view.rs`、`src/ui/terminal/pane.rs`

- [ ] `overlay_damage_rows` 由 overlay range 直接计算行区间，删除 screen-lines × range-count 扫描。
- [ ] semantic damage capture 建立 `viewport_row -> row index`，删除 changed rows × viewport rows 查找。
- [ ] 无 selection 时 input path 不获取 term lock；只有状态实际变化才 `cx.notify()`。
- [ ] 将 input notify 与 parser redraw 合并，避免 echo 前无内容变化的额外整帧。
- [ ] `SmallVec`/Host input ownership 在不复制的前提下进入 bounded queue；协议转换只在必要边界分配一次。
- [ ] render cache 的 text runs 使用不可变共享 ownership，避免全屏 damage 时无条件深 clone。
- [ ] parser/prepaint mutex 架构改造只有在 M5 证明 lock wait 为主要占比时实施；优先缩短临界区和批处理，不先引入双状态真相源。

**验收：** 严格 terminal perf matrix 通过；每项优化对应指标改善且不牺牲 queue 上限、damage 正确性、selection/hyperlink/cursor 行为。

---

# D7：完成发布分发完整性与平台支持

## D7-1：实现应用内下载与 SHA-256 校验

**主要区域：** `src/runtime/update.rs`、`src/ui/workbench/update.rs`、release metadata tests

- [ ] updater 不再只打开浏览器冒充可验证更新；下载到 profile-scoped 临时文件并限制大小、redirect、content type 与最终 URL。
- [ ] 流式计算 SHA-256，与 manifest 中规范小写 hex 比较；不匹配立即删除临时文件并显示错误。
- [ ] 下载完成后原子移动到 staging；安装/打开前再次确认文件存在且 hash 未变化。
- [ ] 覆盖 truncated download、超限、redirect 到非 GitHub、错误 asset、hash mismatch、磁盘满与取消。
- [ ] 在应用内下载完成前，UI/README 只宣称“打开下载页”或提示手动核对 `SHA256SUMS`，不得宣称已验证。

**验收：** 任何损坏、错配或被替换的资产都不能进入安装/打开路径；临时文件在失败与取消后清理。

## D7-2：增加发布者真实性

**主要区域：** release metadata、macOS/Windows packaging、CI secrets policy

- [ ] update manifest 使用独立离线/受保护私钥签名，应用内置公钥并在解析 asset URL/hash 前验证签名。
- [ ] 定义 key rotation、旧 key 撤销与签名版本；签名失败 fail closed。
- [ ] macOS 接入 Developer ID、entitlements、notarytool 与 stapler；保留无 credential 的本地 `--no-sign` smoke。
- [ ] Windows 接入 Authenticode/signtool；签名 credential 仅在受保护 release environment 可用。
- [ ] CI 验证 release 产物签名、notarization/staple 与 Windows signature subject。
- [ ] secrets 不进入 fork PR、日志、artifact 或普通 build job。

**验收：** updater 同时验证 manifest signature 与 asset hash；正式 macOS/Windows 产物通过系统签名验证，缺少或错误签名时 release 失败。

## D7-3：明确并补齐支持矩阵

**主要区域：** `.github/workflows/release.yml`、`src/runtime/update.rs`、packaging scripts、README

- [ ] 产品先明确 Intel macOS 与 Linux arm64 是支持、实验性还是不支持；README、manifest 与 workflow 使用同一矩阵。
- [ ] 若支持，增加对应 release matrix、规范资产名、`platform_asset_key`、打包 smoke 与 updater tests。
- [ ] Linux tarball 提供可执行安装入口或生成正确绝对/相对 desktop launcher，不依赖用户手工把内部 `bin/` 加入 PATH。
- [ ] 移除 `/usr/bin/awk` 硬编码，使用 PATH 工具并检查平台依赖。

**验收：** 每个声明支持的平台/架构都有实际产物、smoke、manifest key 与 updater test；未支持平台得到明确说明而不是错误直链。

---

# 需复现或产品决定后再实施的项目

以下条目不在没有证据时直接改代码；先完成指定 decision/reproduction gate。

| Review 条目 | Gate | 通过后归属 |
| --- | --- | --- |
| 4.10 request_id 回绕 | 计算可达性并决定连接级 overflow contract；不得只说“理论不可能” | C4 |
| 5.3 PTY resize/grid resize | 构造可重复的旧 winsize 输出错位测试并量化频率 | U6 或独立 correctness PR |
| 5.10 Vi 模式输入 | 明确产品只支持 navigation/selection，还是兼容 Alacritty insert | 产品决定 |
| 7.3 project tree 未虚拟化 | 用 M5 证明 `tree()` 实际构建所有行且占比显著 | U6-1 |
| 8.11 OMP outcome 枚举 | 对照当前 OMP 实际 extension contract，记录允许值 | A2 |
| 9.5 额外架构 | 明确支持矩阵 | D7-3 |

---

# 非阻塞清理清单

这些项目不阻塞前述 correctness/security phases，但必须在相关文件被修改时一并 clean cutover，或在 D7 完成前集中清理。

- [ ] `src/main.rs` 两处 `collapsible_if`；不处理 macOS test cfg 产生的 Linux dead_code 假阳性。
- [ ] i18n：Command unavailable、directory picker failure、desktop tray Host 状态全部使用 `UiTextKey`。
- [ ] breadcrumb render 不 clone 整个 breadcrumbs，稳定 element id 不每帧 `format!`。
- [ ] OpenCode/Agent 文本、env name、index entries 统一边界常量与错误。
- [ ] `PortablePtySession` release build 对未 `finish()` 的资源提供 diagnostics，而不只 `debug_assert`。
- [ ] terminal semantic update queue overflow 采用 coalesce/resync，不通过单纯扩大容量处理。
- [ ] atomic SFTP save 的陈旧 temp/backup 清理有测试和 diagnostics。
- [ ] Linux desktop entry 安装后可从 launcher 启动。
- [ ] release/test fixture 的 asset 名称与生产规范完全一致。
- [ ] Login startup、Host SSH 与 terminal perf 的手动 smoke 入口在 release checklist 中明确记录。

---

# 推荐 PR 边界

每个 PR 只包含一个可独立回滚的契约；不得把安全 wire、UI memoization 与发布 workflow 混在一起。

1. `validation-gate`：G0-1。
2. `release-fail-closed`：G0-2。
3. `client-handshake-recovery`：R1-1、R1-2。
4. `host-recovery-timeouts`：R1-3。
5. `agent-launch-attempt`：A2-1。
6. `agent-hook-bounds`：A2-2、A2-3 中相关边界。
7. `host-owned-ssh-credential-binding`：S3-1、S3-2。
8. `sftp-serialization`：S3-3、S3-4。
9. `semantic-scroll-consistency`：C4-1。
10. `host-client-bounded-recovery`：C4-2。
11. `watcher-and-close-queues`：C4-3。
12. `boundary-failures`：C4-4。
13. `ui-perf-baseline`：M5。
14. `project-tree-work-area-cache`：U6-1、U6-2。
15. `palette-file-index`：U6-3、U6-4。
16. `terminal-hot-path`：U6-5，仅包含 profile 证明的项目。
17. `verified-updates`：D7-1。
18. `signed-releases-and-platforms`：D7-2、D7-3。

---

# 阶段验收与最终 Definition of Done

## 每个行为修复 PR

- [ ] 旧实现上可复现失败，新实现通过同一测试。
- [ ] 运行直接受影响 crate/test target，不用全仓库测试替代具体回归。
- [ ] 所有 exported symbol caller 已迁移；没有 deprecated alias、兼容 shim 或旧状态分支。
- [ ] 新增错误 typed、可观察、无 secret；retry、timeout、cancel、drop 顺序均有测试。
- [ ] 新增 queue/cache/journal 有上限与 overflow/eviction tests。

## 每个 UI 变更 PR

- [ ] 使用实际 GPUI surface 驱动交互；验证 selector、视觉快照或 render snapshot，而不只断言 RPC。
- [ ] 测试 content、scrollbar、selection、hit test、focus 与 persistent state 的可观察行为。
- [ ] 保存 before/after perf 输出；结构性 rebuild counter 满足对应 phase 验收。

## 每个 workflow/release PR

- [ ] 在本地运行 workflow 中的核心命令并记录退出码。
- [ ] 测试成功路径与至少一个失败路径；release metadata、资产重复、hash/signature mismatch 必须 fail closed。
- [ ] 不依赖仅存在于开发机的 path、credential、GUI session 或浮动 toolchain。

## 全计划完成

- [ ] `cargo fmt --all --check` 通过。
- [ ] 自有 workspace strict clippy 通过，vendor 不进入自有 warning gate。
- [ ] `cargo test --workspace --no-fail-fast` 在支持环境通过；watcher 环境差异由产品降级而不是 skip 吸收。
- [ ] 三平台 required build/package/process smoke 通过。
- [ ] `scripts/test_release_tools.py` 与 terminal performance strict summarizer 通过。
- [ ] recovery、Agent retry、SSH binding、semantic scroll、watcher fallback、dirty-close、journal churn 均有端到端场景验证。
- [ ] Host/Direct terminal 继续满足既有吞吐、延迟、CPU、memory 门槛。
- [ ] release 对精确 tag validation、CHANGELOG、不可变资产、manifest signature、asset hash 与平台签名 fail closed。
- [ ] README、usage、Host architecture、release checklist、CHANGELOG 与实际能力一致。
- [ ] `docs/reviews/2026-08-31-full-project-review.md` 中每个 high/medium finding 已链接到完成 PR、明确接受的风险或经证据降级的结论；不得静默遗漏。
