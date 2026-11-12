# 移动端项目页与左抽屉

分支 `feat/mobile-project-page`，worktree `.worktrees/mobile-project-page`，基于 master 87a5e22。设计见 `docs/plans/2026-11-11-mobile-client-design.md` §2。

**页面关系**

- 首页（总控）与项目页是平级的根页面，用左抽屉切换，不用推入式导航。
- 左抽屉列出“总控”入口和项目列表；项目带等待、失败、运行数量的角标。最下面是“忘记这台 Host”。

**项目页**

- 顶栏：左抽屉按钮、项目名。
- 终端 tab 横向列表，只放终端，不放文件。
- 当前 tab 有多个 pane 时，才显示“◀ n/m ▶”切换条。
- 终端本身。未接管时底部显示“接管终端”提示；该 pane 的 Agent 在等待时，提示改为“等批准，接管后回复”（或“等回答”）。

**不在这一块**

右抽屉（文件 / 会话）和全屏文件编辑放到下一块，它们需要核心新增读写文件的接口。这一版顶栏不放右抽屉按钮。

每个任务先写失败的测试，再实现，测试通过后提交。编译用 `CARGO_PROFILE_DEV_DEBUG=line-tables-only`，合并后清理构建缓存。

## 任务 1：概览带上 pane（Rust）

改 `crates/yttt-mobile-core/src/overview.rs`：

```rust
pub struct ProjectSummary { pub project_id, pub name, pub tabs: Vec<TabSummary>,
    /// The tab the desktop shows, which the phone opens first.
    pub selected_tab_id: String }
pub struct TabSummary { pub tab_id, pub title, pub panes: Vec<PaneSummary>,
    /// The pane the desktop has focused, shown first.
    pub focused_pane_id: Option<String> }
pub struct PaneSummary { pub pane_id, pub title, pub session_id /* "{project}:{tab}:{pane}" */,
    pub state: PaneState /* Idle | Running | Exited */ }
```

- pane 顺序按布局树深度优先、左边先。标题取 `PaneConfig.title`，状态取该 tab 的 `pane_states`；布局里有、`pane_states` 里没有的 pane 记为 `Idle`。
- 首页计数改为数 `tabs` 与 `panes`。
- 单元测试：分屏 tab 的 pane 顺序与标题、聚焦 pane、选中 tab、缺少状态的 pane 为 Idle。
- 两端首页的单元测试按新类型更新。

## 任务 2：项目页的纯逻辑（两端，单元测试）

- `ProjectBadges(overview)`：每个项目的等待数、失败数、运行数（Working、Starting、Idle）。
- `PaneCursor`：在一个 tab 的 pane 之间前后切换，到头不再动；起点是聚焦的 pane，没有就是第一个。
- `tipText(agent)`：没有 Agent 或 Agent 没在等待时为“接管终端”；等批准时为“等批准，接管后回复”；等回答时为“等回答，接管后回复”。
- 打开一个 Agent 时定位到它的项目、tab 和 pane。

iOS 用 XCTest，Android 用 JVM 单元测试。

## 任务 3：Android 项目页与左抽屉

- 根页面状态：`Home` 或 `Project(projectId, tabId, paneId)`。返回键在项目页回首页。
- 左抽屉：`ModalNavigationDrawer` 加 Material 3 的 `NavigationDrawerItem`；角标用 `Badge`。
- tab 列表用 `PrimaryScrollableTabRow`（或 `ScrollableTabRow`）。
- 切换条用两个 `IconButton` 加“n/m · pane 标题”。
- 终端：把现有的 `TerminalScreen` 改成可嵌入的 `TerminalPane`，去掉“返回”，提示文字由外面传入。pane 没在运行（没有对应终端）时显示“这个 pane 没有在运行”。
- 首页点 Agent，进入它的项目页并选中对应的 tab 和 pane。点等待通知也走同一条路。

## 任务 4：iOS 项目页与左抽屉

- 根页面同上。抽屉是自写的容器：从左侧滑出，带遮罩，点遮罩或向左拖关闭，从屏幕左缘约 20 pt 内向右拖打开；内容用系统 `List`。
- tab 列表是横向 `ScrollView`，每个 tab 一个按钮，选中的高亮。
- 切换条、终端、提示与 Android 相同。终端沿用 `TerminalHost`。

## 任务 5：spike Host 与模拟器验证

- spike_host 加一个分屏 tab（两个 pane），以验证切换条。
- Android：用 uiautomator 打开抽屉、点项目、切 tab、用“▶”切 pane、在等待中的 Agent pane 上看提示文字，截图。
- iOS：模拟器不能点，加自动化参数（打开项目、tab、抽屉开着）后截图检查布局。
- 截图存本地 scratchpad，不上传。

## 任务 6：结论

更新移动端设计 §2（实现与偏差），在本文末尾写执行记录。
