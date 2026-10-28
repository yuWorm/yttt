# yttt C/S 架构问题分析(对照 herdr)

- 日期:2026-10-28
- 基线:yttt `master@bd8f930`(0.3.18);herdr `2713feb5`(0.9.5,2026-10-27)
- 用途:约束 [`2026-10-28-cs-convergence-plan.md`](./2026-10-28-cs-convergence-plan.md) 的方向。后续 C/S 改动若与本文结论冲突,先修订本文再改代码。

## 1. 结论

yttt 的 C/S 问题不在于"做了 C/S",而在于四个设计选择叠加:

1. 一个 Client 拆成多条 socket,消息之间没有先后顺序;
2. 终端按字节流同步,断号由 Client 主动拉 checkpoint;
3. Host 只接受完全相同的协议版本,build 不同就替换 Host;
4. 布局归 Client、终端归 Host,两边要持续对账。

herdr 在这四处都用了更简单的做法。按"收益 ÷ 改动量"排序,依次借鉴:协议分层与兼容 → 服务端维护基线 → 单连接 → 布局归 Host。

## 2. 现状数据

2026-08 起 21 个 C/S 修复提交(按 `git log` 关键词筛选、逐个归类):

| 类别 | 提交数 | 新增行 | 代表提交 |
|---|---:|---:|---|
| 恢复 / 生命周期 | 9 | 11,231 | `1bb6128` `5f79bb3` `2e761f8` |
| 迟到事件需要加栅栏 | 2 | 579 | `68b0dff` `3ce53b7` |
| 输入 / 帧延迟 | 6 | 2,846 | `483e313` `9cdf48f` `13973bc` |
| 控制权交接 / 工作区发布 | 4 | 1,721 | `598406a` `3c33b84` |

资源协议在 0.3.14–0.3.17 之间从 v18 升到 v21,每次 CHANGELOG 都要求"桌面、Host、远端 Server 与 CLI 需同步更新"。

## 3. 结构差异

| 维度 | yttt | herdr |
|---|---|---|
| 每个 Client 的连接数 | 固定 control、interactive、state-events 三条,桌面再加 lifecycle/desktop-owner;每个已附加终端再加一条 terminal-data。每条单独握手、单独校验 `host_epoch`(`crates/yttt-client-core/src/lib.rs:893-905,1626-1667`) | 一条。服务端发送侧两个队列:有序控制队列 + 容量为 1 的渲染队列,满了就丢(`src/server/client_transport.rs:120-127`) |
| 终端同步 | 字节流:delta 必须满足 `base_sequence == mirror.sequence`,断号后 Client 拉 checkpoint | 状态:服务端记住每个 Client 已成功收到的画面(基线);发送队列满时不推进基线,下次补发全量(`src/server/headless/retained_surface.rs:571-597`) |
| 推送范围 | 所有已附加终端;只在 pane Drop 时 detach(`src/ui/terminal/pane.rs:2034-2040`) | 只给 Client 推它看得见的 pane(`src/server/headless/surface_interest.rs:9-33`) |
| 布局归属 | tab/pane 归 Client,Host 只按 CAS 存 manifest;因此需要 profile 单写入者、控制权交接、ControlLost、设备恢复副本 | workspace/tab/pane 都在服务端 `AppState`;Client 发操作意图,多个 Client 各看各的 |
| 版本兼容 | Host 只接受完全相同的协议版本(`ProtocolRange::exact`,`crates/yttt-host/src/lib.rs:232-233`);build 不同就替换 Host,Host 忙则失败(`src/host_launcher.rs:284-313,637-640`) | 内部协议 v22 可随时改;TUI 走独立的稳定协议(generation 1),新旧版本可互连(`AGENTS.md:82-94`) |
| 状态校验计数器 | 十余种:host/session/control/lease/geometry epoch、sequence、worker generation、Agent 的 (epoch, generation, sequence)、catalog revision、start_id… | `boot_id` + 两个 revision;事件流落后太多直接返回 `events_lost`,客户端重拉快照(`src/api/subscriptions.rs:326-337`) |

## 4. 根因与可借鉴做法

### 4.1 协议分两层:稳定层可独立演进(→ 计划 C1)

- 文档与代码不一致:`docs/wire-evolution.md` 规定按版本区间协商、build 指纹只用于诊断;实际 Host 只接受完全相同的版本,launcher 只要 build 不同就会尝试停掉 Host(`ExistingHostAction::ReplaceBuild`)。`RejectReason::BuildMismatch` 没有任何生产者。
- Host 收到解不开的请求(例如新 Client 发来的新请求)直接断开 control 连接(`crates/yttt-host/src/lib.rs:1104`);Client 收到解不开的事件同样断线重连。因此任何追加都必须升协议版本,任何升版本都要求全部组件同步升级。
- 借鉴 herdr:同 build 的进程间可以随意改,跨版本的连接只做追加演进;对方不认识的请求返回带类型的 `Unsupported`,只禁用这一个功能;已发布的消息标签与字段冻结在测试样本里(herdr `tests/fixtures/endpoint-method-shapes-v1.json`、`src/protocol/wire.rs:1957-2108`)。
- launcher 只要协议能协商成功就直接 attach。这是让 Independent Host 撑过桌面升级的前提。

### 4.2 终端同步改成"服务端维护基线",只推可见终端(→ C2)

- Host 为每个"Client × 终端"保存上一次成功发出的画面。能发时拿它做 diff;发不出去就不推进基线,之后补发全量。Client 永远不会遇到断号。
- 可删除:checkpoint 请求通路、`SequenceGap` 处理、worker generation、每终端一条连接及其"最多 4 条并行"限制。
- Client 上报每个窗口看得见哪些终端;看不见的终端只发 title、进程、退出、Agent 事件,切回时发一屏全量。
- 代价:多个 Client 看同一终端时不再共用编码好的帧(`docs/host-client-architecture.md:382-384`);每个可见终端在每个 Client 上多存一份画面。

### 4.3 一条连接,内部分逻辑通道(→ C3,依赖 C2)

- 响应走 control、状态事件走 state-events、终端输出走 terminal-data,三条 socket 之间没有顺序保证。0.3.16 修的"旧会话退出事件清掉新会话视图""启动响应先于晚到的进出事件"都属于这种乱序,当时只能再加一层栅栏。
- 每个 Client 一条连接,帧头带逻辑通道 id。Host 发送侧:一个有序可靠队列(响应、状态事件、终端生命周期事件,同队列天然有序);每终端一个"只保留最新一帧"的渲染槽;输入优先发送。
- 慢请求由 Host 并发执行,大响应分块发送(herdr `ClientShellEndpointResponseChunk`),不需要为此拆 socket。
- [推断] 8 月那批输入/帧延迟问题可能源于 Host 发送路径锁粒度过大,需用现有 perf-metrics 复现后再定。

### 4.4 布局归 Host(收益最大,改动最大 → C4)

- 现状:Host 管终端,Client 管 tab/pane,Host 再持久化 Client 发来的 manifest。由此需要 catalog 与布局对账、控制权交接(Preparing/Fencing)、5 秒强制确认、ControlLost、设备恢复副本。§2 中"恢复 / 生命周期"与"控制权交接 / 工作区发布"两类共 13 个提交、约 1.3 万行,大多落在这里。
- herdr:布局是服务端共享事实,Client 只发操作意图(split、close、move),服务端按序执行并广播新布局与 revision;看同一个 tab 时最后操作的 Client 决定尺寸(`docs/.../concepts.mdx:69-73`)。
- 方向:Host 持有 workspace 树与 pane→session 绑定;Client 只保留 focus、选区、滚动位置、主题和编辑器 UI 状态。profile 级单写入者与交接协议改为降级或删除。
- 难点:编辑器草稿改为按单个文档 revision 存在 Host(乐观并发),不再和整个 workspace 的 CAS 绑定。需单独设计。

### 4.5 升级时保活进程(可选,C1 之后再评估)

herdr `--handoff` 用 `SCM_RIGHTS` 分批(每批 64 个)把 PTY fd 交给新 server,分阶段提交、失败回滚(`src/server/handoff.rs`),仅 Unix,仍标 experimental。C1 完成后 Independent Host 已能撑过桌面升级,handoff 推后。

## 5. 不照搬的部分

- 服务端整屏渲染:herdr 是 TUI,服务端直接排出整屏单元格。yttt 是 GPUI + 编辑器,布局与渲染必须在 Client。借的是同步模型:基线、可见性、只保留最新帧。
- 本机安全模型:herdr 本机只靠 0600 socket。yttt 有 TLS/SSH 远程入口,现有认证保留;单连接后每个 Client 只需认证一次。
- herdr 自身的复杂点:画面复用、delta、滚动共用一个基线并原子校验;切换连接对象有 5 秒回滚(`src/client/endpoint/activation.rs`)。只借最小可用部分。

## 6. C/S 之外值得借鉴

- Agent 状态检测:TOML 规则识别屏幕内容、可热更新,`agent explain` 说明命中哪条规则(`src/detect/`)。yttt 目前只靠 hook,可作为 hook 失效时的兜底。
- 面向 Agent 的 API:`events.wait`、`pane.wait_for_output`,prompt 后确认 Agent 真正进入 Working(`src/api/wait.rs`);事件中心只保留 512 条,落后的订阅方收到 `events_lost` 后重拉快照。
- 持久化兜底:session 文件读取失败时先备份原文件(保留 3 份);每 15 分钟多存一份布局历史,最多 48 份(`docs/.../session-state.mdx:39-47`)。
- 工程纪律:"每字节 × 窗格数 × 客户端数"这类会成倍放大的路径要单独做性能评审。

## 7. 证据索引

herdr 路径相对 herdr 仓库根(本机副本位于 `sources/herdr`,不入库)。yttt 路径相对本仓库根,行号以 `bd8f930` 为准。
