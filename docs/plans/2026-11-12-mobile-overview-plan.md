# 移动端首页概览：数据与首页

分支 `feat/mobile-overview`，worktree `.worktrees/mobile-overview`，基于 master 267ed5d。

这是正式 App 的第一块：首页（总控）。设计见 `docs/plans/2026-11-11-mobile-client-design.md`：

- §2 首页：数量、“需要处理”、运行中、已结束；
- §3 分层：Rust 只交出去抖后的快照，数数与排序由原生做；
- §5 首页数据：Agent 快照概要，不订阅 board。

每个任务先写失败的测试，再实现，测试通过后提交。编译用 `CARGO_PROFILE_DEV_DEBUG=line-tables-only`，减小 `target`。

## 任务 1：概览模型（纯函数）

`crates/yttt-mobile-core/src/overview.rs`：从工作区布局、Agent 快照与资源目录构建 `Overview`。

```rust
pub struct Overview {
    pub projects: Vec<ProjectSummary>,
    pub agents: Vec<AgentSummary>,
    /// Terminals the Host runs (the resource catalog).
    pub terminals: u32,
}
pub struct ProjectSummary { pub project_id: String, pub name: String, pub tabs: Vec<TabSummary> }
pub struct TabSummary { pub tab_id: String, pub title: String, pub panes: u32 }
pub struct AgentSummary {
    pub session_id: String,          // "{project}:{tab}:{pane}"
    pub project_id: String, pub tab_id: String, pub pane_id: String,
    pub provider: String,
    pub state: AgentState,            // AgentSnapshot::view_state()
    pub waiting: Option<WaitingKind>, // Approval / UserInput / External
    pub waiting_message: Option<String>,
    pub task: Option<String>,
    pub current_action: Option<String>,
    pub state_started_at_ms: u64,
    pub updated_at_ms: u64,
}
```

- `AgentState` 与 `WaitingKind` 镜像 `AgentViewState` 与 `WaitingReason`，都是 uniffi 枚举。
- 项目名取 `layout.project.name`，tab 标题取 `TabConfig.title`，pane 数取该 tab 的 `pane_states` 个数。
- Agent 的 project/tab/pane 用 `AgentSnapshotUpdate.scope`。它在布局里找不到（例如属于别的工作区）时照样列出，名字留给原生用 id 显示。
- 不排序、不计数：只按 Host 的顺序列出，数数和“需要处理在前”交给原生。
- 单元测试：用手写的布局与快照，覆盖字段映射、找不到布局的 Agent、没有布局时只有 Agent。

## 任务 2：会话接线与推送

- `MobileSession` 保存最近的工作区布局：来自 `open_active_workspace` 的 `Opened.layout`，之后随 `ServerEvent::WorkspaceChanged` 更新。`AgentStateCleared` 不需要单独处理，Agent 快照由 client-core 维护。
- `MobileSession::overview()` 由布局、`client.agent_snapshots()`、`client.resource_catalog()` 现算。
- `MobileSession::watch_overview(callback)`：后台任务监听这些 `ClientEvent`，有变化时去抖 250 ms 调用回调一次。
  - 监听：`AgentSnapshotUpdated`、`AgentSnapshotsReset`、工作区变化、资源目录变化、重新连上。
  - 去抖期间的多次变化合并成一次。
  - 第一次调用立即给出当前快照。
- 确认手机连接不请求 board（`accepts_agent_board_interest` 且从不订阅），Agent 快照带 `board_omitted`。
- 集成测试 `the_overview_follows_an_agent_into_waiting`：
  - 真 Host 上起一个终端，用 Host 注入的 hook 环境变量依次发 Codex 的 `SessionStart`、`UserPromptSubmit`（带 prompt）、`PermissionRequest`；
  - 断言回调最终给出该 Agent：`state == Waiting`、`waiting == Approval`、`task` 为 prompt，项目名与 tab 标题来自布局；
  - 断言快照没有 board；
  - 断言一连串变化只触发少数几次回调（去抖）。

## 任务 3：FFI

- `Overview` 等类型导出为 uniffi Record/Enum。
- `HostSession.overview()` 是同步的，现算。
- `HostSession.watch_overview(listener: OverviewListener)` 回调 `on_overview(Overview)`，在 Rust 线程上调用，原生自行切回主线程。重复调用会替换之前的监听；`disconnect` 时停止。
- 原生连上后先 `open_active_workspace()`（FFI 新增），再开始监听。

## 任务 4：首页（两端）

spike App 的终端列表换成首页。只用系统组件：iOS 用 `List` 分组，Android 用 `LazyColumn` + Material 3 `ListItem`。

- 顶部：Host 名、连接状态，以及项目、tab、终端、Agent 的数量。
- “需要处理”：等批准 → 等回答 → 其他等待 → 失败，各组内最近变化的在前。显示项目名、tab 标题、任务、等待说明、持续时间。
- “运行中”：Working、Starting、Idle。
- “已结束”：Completed、Interrupted、Stale，默认折叠。
- 点一个 Agent 打开它的终端。正式版进项目页，项目页是下一块。
- 底部保留“所有终端”入口（原列表），方便调试。

## 任务 5：spike Host 的假 Agent 与模拟器验证

- `spike_host` 加一个终端 `spike:agent:1`，用 shell 加 `curl` 向 Host 注入的 hook 地址循环发事件：开始会话 → 提交任务 → 工作（`PreToolUse`）→ 等批准（`PermissionRequest`）→ 完成，每步停几秒。这样首页能看到状态变化。
- 两个模拟器上看首页：数量正确、等批准的 Agent 出现在“需要处理”、状态随时间变化、点进去打开终端。截图存本地 scratchpad，不上传。

## 任务 6：结论

更新移动端设计 §5 首页数据（实现与偏差），在本文末尾写执行记录。合并后清理构建缓存。

## 不做

- Android 通知与前台服务（下一块，依赖本块的概览数据）。
- 项目页、抽屉、文件（之后几块）。
- 用 Agent 命令直接回答问题（`AgentCommand`），首版只跳到终端。

## 执行记录（2026-11-12）

| 任务 | 提交 | 结果 |
|---|---|---|
| 1 概览模型 | 09a3df3 | 单元测试 4 项；去掉“等待原因只在 Waiting 时算”后对应测试失败（变异检查）。 |
| 2 会话接线 | 7769e8c | 集成测试：真 Host 打开项目、Host 启动 Agent pane，经它的 hook 地址发 Codex 的 `SessionStart`、`UserPromptSubmit`、`PermissionRequest`，回调给出等批准的 Agent 与任务，三次连续变化只推了不超过 3 次，快照都没有 board。 |
| 3 FFI | 950b3e6 | `open_active_workspace`、`overview`、`watch_overview` + `OverviewListener`。 |
| 4 首页 | 89fdfa5 | 两端首页；分组排序是原生代码，iOS XCTest 与 Android JVM 单元测试各 2 项。 |
| 5 假 Agent 与验证 | 本提交 | 见下。 |

### 偏差

- **Agent 的 tab 与 pane 不能用 hook scope。** 集成测试发现，Host 自己启动的 pane 的 `AgentHookScope` 里 `tab_id` 与 `pane_id` 都是整个终端名（`web:agents:codex`）。改为先按布局里的 `{project}:{tab}:{pane}` 匹配终端名（与桌面按终端名找 pane 一致），找不到再去掉 `{project}:` 前缀按第一个冒号拆。单元测试的夹具也改成这种 scope。
- 测试里的桌面端要发 `SetActive`，工作区才会成为 active；手机仍然不发。
- iOS 测试 target 不依赖 App，`HomeSections.swift` 与生成的绑定一起编进测试 target。
- spike_host 改为经工作区打开项目“Spike”（由 Host 启动 pane），终端名不变（`spike:shell:1`、`spike:demo:1`），另加两个用 `curl` 发 hook 的假 Agent（`spike:review:codex`、`spike:build:codex`）。

### 模拟器验证

- iOS：首页显示 1 项目、4 Tab、4 终端、2 Agent；状态随假 Agent 变化：Build 在“需要处理”显示“等批准 · Fix the failing build”，Code review 在“运行中”显示“工作中 · Bash npm test”，之后两个都进“已结束”。
- Android：同样的数量与状态变化；用 uiautomator 找到“Spike · Build”这一行并点击，打开了它的终端（显示假 Agent 的输出）。
- 编译用 `CARGO_PROFILE_DEV_DEBUG=line-tables-only`，本 worktree 的 `target` 为 8.4 GB（之前同类 worktree 20–51 GB）。
