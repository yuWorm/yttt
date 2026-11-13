# 移动端：按平台原生方式重做界面

分支 `feat/mobile-native-ui`，worktree `.worktrees/mobile-native-ui`，基于 master 47bdcd8。

**决定（2026-11-13）**

- 界面全部用系统风格，交互也按各平台的习惯来。
- 取消侧滑抽屉，这推翻了 2026-11-11 的“两端都用侧滑抽屉”：
  - **iOS**：底部标签栏（总控 / 项目 / 设置）加推入式导航。文件和会话放在 sheet 里。
  - **Android**：底部导航栏（总控 / 项目 / 设置）加返回栈，支持预测性返回。文件和会话放在底部弹出的 sheet 里。

**不变的**

- 首页是总控，内容不变：数量、需要处理、运行中、已结束。
- 项目页只放终端 tab，一个 tab 有多个 pane 时一次只看一个。
- 文件全屏编辑，不做文件 tab、分屏和 Vim。
- 通知只在 Android 上做。
- 业务逻辑都已有测试，这次不动：总控排序、项目定位、文件树、Host 列表、等待通知、按键编码。

每个任务先写失败的测试（只针对新的纯逻辑），再实现；界面在模拟器上截图验证。合并后清理构建缓存。

## 页面与交互

| 页面 | iOS | Android |
|---|---|---|
| 根 | `TabView`，三个标签各自一个 `NavigationStack`；“总控”标签上的角标是需要处理的数量 | `Scaffold` + `NavigationBar`，三个顶层目的地；“总控”上的 `BadgedBox` 显示需要处理的数量；Navigation Compose 管返回栈 |
| 总控 | 大标题；`insetGrouped` 列表：Host 行（名字、连接状态，点开是切换 Host 的菜单），四个数量，“需要处理”“运行中”，“已结束”可折叠；下拉刷新；没有 Agent 时显示 `ContentUnavailableView` | `TopAppBar` 随滚动收起；Host 行同上（`DropdownMenu`）；数量放在一张卡片里；Agent 用 `ListItem`；`PullToRefreshBox`；空状态带图标 |
| 项目列表 | 列表，每行是项目名加等待、失败、运行中的数量，点开推入项目页 | 同左，`ListItem` + `Badge` |
| 项目页 | 推入后隐藏标签栏，留给终端。标题是项目名，标题菜单（`toolbarTitleMenu`）列出各 tab 和当前 tab 的 pane，当前项打勾。右上角“文件”按钮打开 sheet。接管后右上角出现“交还电脑” | 隐藏底部导航栏。`TopAppBar` 带返回、项目名、“文件”按钮；下面是 `PrimaryScrollableTabRow`（终端 tab），有多个 pane 时再加一行 `SecondaryScrollableTabRow`（pane），这正是 Material 3 的二级标签。接管用 `ExtendedFloatingActionButton`，带键盘图标；接管后顶栏出现“交还电脑” |
| 文件与会话 | sheet，可停在半屏或全屏；顶部是分段控件（文件 / 会话），文件树可展开，点文件后关掉 sheet，全屏打开编辑器 | `ModalBottomSheet`，顶部 `PrimaryTabRow`（文件 / 会话），树形列表带文件夹和文件图标，点文件进入编辑器页面 |
| 编辑器 | `fullScreenCover`，左“取消”、右“保存”，保存结果显示在底部工具栏 | 独立页面，`TopAppBar` 左边是关闭图标，标题是文件名，副标题是路径，右边是保存图标；结果用 `Snackbar`；返回手势遇到未保存的修改先问 |
| 设置 | `Form`：Host 列表（打勾的是当前，点一下切换，左滑忘记，忘记前确认）、“添加 Host”（sheet）；关于（版本、开源许可） | Host 列表（当前的打勾，菜单里“忘记”，先确认）、“添加 Host”；后台运行（电池优化）、通知设置（跳到系统设置）；关于（版本、开源许可页） |
| 配对（还没有 Host 时） | `Form`，同现在 | `Scaffold` + `TopAppBar`；扫码、从图片识别、粘贴配对码 |

- 点 Agent、点通知，都推入它所在的项目页并定位到那个 pane；不在任何 pane 里的终端单独推入一个终端页。
- 终端右上角的统计（帧率、流量、drain 耗时）只在调试版显示；正式版不显示。
- 连接状态（已连接 / 正在连接 / 连接失败，直连还是经中继）显示在总控的 Host 行里。
- 两端都跟随系统的深浅色。Android 12 起用动态取色，内容延伸到状态栏和导航栏下面。

## 任务 1：导航目标（纯逻辑，先写测试）

- 两端各写一个 `destination(agent, overview, terminals)`：
  - Agent 在某个 pane 里 → 项目页，定位到那个 pane；
  - 不在 pane 里，但终端还在 → 单独的终端页；
  - 终端已经没了 → 返回空，界面提示“这个终端已经不在了”。
- Android 的路由参数编码（项目 id、tab、pane、文件路径里可能有 `/` 和空格）：`Routes.project(place)`、`Routes.editor(projectId, path)`，以及反向解析。用 `java.net.URLEncoder`，JVM 单元测试测往返。

## 任务 2：Android

- 依赖：`navigation-compose` 2.9.8、`material-icons-extended` 1.7.8（正式版经 R8 去掉没用到的图标）。
- `AndroidManifest`：`enableOnBackInvokedCallback="true"`；`MainActivity` 调 `enableEdgeToEdge()`。
- `YtttTheme`：Android 12 起用动态取色，否则用 Material 3 的默认配色，深浅色跟随系统。
- 按上表重写各页面。`MainActivity` 只剩生命周期、权限、导航宿主；页面拆到各自文件。
- 删除左右抽屉（`LeftDrawer`、RTL 的右抽屉）。
- 调试统计：`debug` 源码集里 `SHOW_DIAGNOSTICS = true`，`release` 里为 `false`。

## 任务 3：iOS

- `AppModel`：去掉 `root`、`drawerOpen`、`drawerFilesOpen`；新增 `tab`（当前标签），`overviewPath`、`projectsPath`（两个导航栈），`open(agent:)` 切到总控标签并推入目标页。
- 按上表重写各页面。删掉 `Drawer.swift`，`FilesDrawerView` 改为 sheet 里的 `FilesView`。
- 新增开源许可页：SwiftTerm、Runestone（都是 MIT）。
- 调试统计和自动化参数仍只在 `#if DEBUG` 里。
  - 自动化参数跟着新结构调整：去掉 `DRAWER`；`FILES` 改为打开文件 sheet；新增 `YTTT_DEBUG_TAB_BAR=overview|projects|settings`，选中某个标签。
  - 测试相应更新。

## 任务 4：iOS 界面测试（XCUITest）

模拟器不能用 `simctl` 点击，菜单、sheet、左滑这些交互一直没验证过。新增 UI 测试 target `YtttUITests`：

- 由环境变量 `TEST_RUNNER_YTTT_PAIR_CODE` 传入 spike Host 的配对码，测试再把它交给 App 的 `YTTT_DEBUG_PAIR_CODE`；
- 真的点击：总控 → 点 Agent → 项目页 → 标题菜单换 tab → 打开文件 sheet → 打开文件 → 取消 → 返回；项目标签；设置里的 Host 列表；
- 每一步截图，作为测试附件。
- 不放进默认的测试 scheme：没有 spike Host 时它不能跑。单独一个 scheme `YtttUITests`。

## 任务 5：验证与结论

- 两端单元测试；两端调试版和正式版构建；`check-release.sh` 通过。
- Android 模拟器上点一遍：
  - 每个页面截图，深色模式也截一张；
  - 预测性返回；
  - 通知点开后定位正确；
  - 正式版里没有调试统计。
- iOS 用任务 4 的 UI 测试走一遍，看截图。
- 更新移动端设计 §2（界面）；在本文末尾写执行记录。

## 不做

- iPad 专门的布局（侧边栏、分栏）：iOS 17 的 `TabView` 在 iPad 上就是顶部标签；要侧边栏得等最低版本升到 iOS 18。
- Android 平板、折叠屏的自适应布局（导航栏换成导航轨）。
- 全局搜索、文件搜索。
- 动画、转场的自定义：只用系统默认的。
