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

## 渲染图确认后的修改（2026-11-13，第二版）

渲染图在本地 `scratchpad/native-ui/index.html`，用户确认了第二版。下面几条覆盖上表：

- **图标**：Agent 用桌面版已有的 logo（`assets/icons/agent-*.svg`，Lobe Icons、Oh My Pi，MIT）。
  - 出现在总控的 Agent 行、项目列表、换项目菜单、终端 tab、会话列表和 Android 通知上。
  - Codex、Claude 是彩色的，其余是单色，跟随文字颜色，和桌面一致。
  - 状态带图标：等批准是举手，等回答是对话气泡，工作中是闪电，已结束是对勾。数量、Host、文件类型、设置项也都带图标。
- **项目页**：
  - 顶部标题是项目名，点开就地换项目，列出各项目正在跑的 Agent 的 logo 和等待数。iOS 用 `toolbarTitleMenu`，Android 用标题的下拉菜单。
  - 下面一排是终端 tab，两端都直接显示。Agent 的 tab 带它的 logo，有 Agent 在等的 tab 带一个点。iOS 用系统样式的胶囊按钮，Android 用带图标的 `PrimaryScrollableTabRow`。
  - iOS 这一排的最后一个按钮打开 tab 总览 sheet：每个 tab 一张卡片，显示图标、标题、Agent 状态和 pane 数。终端缩略内容要核心支持，这次不做。
  - pane 切换是最底部一条约 26pt 高的细条，紧贴接管栏的上方，接管后则在按键栏上方。只有一个 pane 时不出现，可以点名字也可以左右滑。Android 去掉第二排 tab。
  - iOS 的接管提示改成贴底的接管栏；Android 仍用 FAB，浮在 pane 条上方。
- **总控的 Host 菜单**：只有切换、添加、“管理 Host”（跳到设置）；忘记 Host 只在设置里做，可以忘记任意一台。
- **配对页**：配对码输入框带粘贴按钮。

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

## 执行记录（2026-11-13）

| 内容 | 提交 | 结果 |
|---|---|---|
| Agent logo | bb4a6ac、8ab0466 | 生成脚本；第二个提交见下文“发现的问题”第 2 条 |
| Android 纯逻辑 | 0c371f6 | tab 标记、项目标记、点 Agent 去哪、路由编码：6 项新测试在桩下失败，实现后通过 |
| Android 界面 | 641b1d4 | 按第二版渲染图重写；抽屉删除 |
| Android 修复 | b1fb3de、8ab0466、7782e75 | 见下文“发现的问题” |
| iOS 纯逻辑 | f71eac4 | 同 Android 的三组测试，桩下失败，实现后通过 |
| iOS 界面与 UI 测试 | a085a2e | 按第二版渲染图重写；`YtttUITests` |

测试数：Android debug 38、release 36；iOS 单元 24，UI 测试 1 个（15 步）。

### 发现的问题（都已修）

1. **Android：存储文件没落盘就改名**。上次直接关掉模拟器后，`hosts.json` 是空文件，App 每次启动都在读它时崩溃。现在写入后先同步到磁盘再改名，Host 列表和设备密钥都是；读不出来的 Host 列表当作没有。
2. **Android：Codex logo 让后台服务崩溃**。SVG 允许弧线参数连写（`0 000 4.496`），Compose 能画，但通知加载大图标走的是系统解析器，它不接受。转换脚本现在把每个数分开写，并检查每个命令的参数个数。
3. **iOS：连接途中换了 Host，连上的还是旧 Host，或者谁也没连上**。现在连上后先核对 Host，不对就断开，改连当前的。
4. **两端：已断开的连接还会送来总控数据**，旧 Host 的 Agent 会混进来。现在只收当前连接的。
5. **iOS：已断开的连接上请求失败，错误一直挂在首页**。现在不再显示这类错误。
6. **iOS：Host 菜单一打开就被关掉**。总控数据频繁更新，菜单跟着重建就会关闭。Host 行改成只在 Host、连接状态变化时才重画。
7. **iOS：在标题菜单里选当前项目，页面会跳回电脑选中的 tab**。现在选当前项目什么也不做。
8. **iOS：系统按钮显示英文**，比如操作表的 Cancel。App 声明了中文为开发语言。
9. **iOS：“全部 Tab”按钮随 tab 一起滚到屏幕外**。现在固定在右端。
10. **Android：编辑器的“已保存”被键盘挡住**。Snackbar 移到了键盘上方。

### 验证

- **Android**（模拟器，调试版和正式版；正式版用调试密钥临时签名）：
  - 总控、项目、设置三个标签；
  - 点 Agent 推入项目页，标题菜单换项目；
  - Server tab 的两个 pane：底部 pane 条切换；接管后 pane 条在按键栏上方，“交还电脑”在顶栏；
  - 文件 sheet；编辑器改一行并保存，电脑上的文件变了；
  - 后台时的等待通知带 Codex 的 logo 作为大图标，点开进入那个 tab；
  - 深色模式和动态取色；
  - 从左边缘右滑返回，在总控按返回退出 App；
  - 正式版：界面正常，终端上没有调试统计。
- **iOS**（模拟器，`YtttUITests` 真点）：
  - Host 菜单；点 Agent 推入项目页，此时标签栏隐藏；
  - 标题菜单；tab 切换、pane 条；tab 总览 sheet；
  - 接管后输入 `echo uitest`，电脑上执行了，然后交还电脑；
  - 文件、会话、编辑器；
  - 项目、设置；左滑忘记一台 Host，在操作表里确认。
- **构建**：两端正式版都通过 `check-release.sh`。体积 Android 22 MB，iOS 30 MB，加了图标库后基本没变。

### 未验证

- Android 13 的主题图标（单色层）。
- iOS 操作表的“取消”是否已显示为中文：改了开发语言后，再跑测试时已经没有第二台 Host 可以忘记，所以没再出现这个操作表。
- 真机；iPad 和 Android 平板的布局。

### 留下的问题

- 核心：连接 Host 的请求可能卡住二十多秒。复现方法是 App 刚启动、还在连接时，对同一台 Host 重新配对。界面已经能处理这种情况（问题 3、4），但卡住本身在 Rust 核心里，需要另查。
