# 项目文件树与文件编辑器设计

Date: 2026-07-10

## 背景

yttt 当前是 project-first、terminal-first 的 GPUI workbench：

- 左侧 sidebar 展示已打开项目。
- 顶部 project tabs 来自 `ProjectLayout.tabs`，每个 tab 承载一个终端 split tree。
- `Workspace` 和 `OpenedProject` 只维护项目、终端 tab、pane 与进程状态。
- `src/ui/editor` 已经有 `CodeEditorState`、语言目录、自动语言识别和
  `gpui-component::InputState::code_editor` 适配，但仅服务 layout/settings 等内置配置编辑器。
- `AppSettings.editor` 目前只有语言识别和 LSP 预留项，没有实际影响项目文件编辑器的显示设置。
- 左侧项目 sidebar 只能折叠，宽度固定为 216px。

本阶段把已有编辑器底座扩展为真正的项目文件编辑体验，同时保留现有终端 layout 的配置语义。
设计参考 Zed 的 item/tab、project panel、buffer settings 和 dock resize 思路，但不复制 Zed 的完整
workspace、buffer、filesystem、watcher 或 LSP 架构。

## 已确认的产品决策

1. 终端布局和文件编辑器使用同一条顶部 Tab 栏。
2. 每个 Tab 是一个运行时工作项：终端布局或项目文件。
3. 文件树位于最右侧；左侧仍是已打开项目列表。
4. Tab 栏尾部固定一个文件夹按钮。Tab 列表独立滚动，按钮不随 Tab 滚走。
5. 文件夹按钮始终可见；文件树打开时按钮显示激活态，关闭时取消激活态。
6. 文件树是否在项目打开时默认显示由设置控制。
7. 左右 sidebar 都支持拖拽宽度；左栏保留折叠态，右栏关闭时宽度为零。
8. 本阶段文件树只负责导航，不提供新建、重命名、移动和删除。
9. 文件默认手动保存，并提供可配置 autosave。
10. 切换项目不关闭文件 Tab。每个项目恢复自己的 Tab、当前文件、文件树展开状态、可见状态和宽度。
11. 关闭项目或应用时必须保护未保存文件。
12. 本阶段不恢复应用重启前的文件 Tab 会话。

## 目标

- 浏览所选项目的文件树并按需展开目录。
- 从文件树打开文本文件，并在统一 Tab 栏中激活或关闭它。
- 同一项目内，同一个规范化路径最多只有一个文件 Tab。
- 编辑、手动保存和自动保存项目文件。
- 显示并保护未保存状态。
- 在磁盘内容发生外部变化时避免静默覆盖。
- 在项目之间切换时保留每个项目独立的编辑会话。
- 让左右 sidebar 共享可靠的拖拽、clamp 和宽度持久化机制。
- 增加一组精简但真实生效的编辑器和 project panel 设置。
- 保持现有终端 tab、split、进程和 layout 配置行为不回归。

## 非目标

- 文件或目录的新建、重命名、移动、复制、删除。
- 编辑器分屏、preview tab、pinned tab 或 Tab 拖拽排序。
- 实时文件系统 watcher。
- 二进制、十六进制或非 UTF-8 编辑器。
- 超大文件编辑器；本阶段单文件上限为 10 MiB。
- minimap、breadcrumbs、sticky scroll、code lens、completion、hover 或 LSP runtime。
- Save As。
- 跨应用重启恢复打开文件、光标或滚动位置。
- 复制 Zed 的通用 Pane/Item、buffer store 或 project store。

## 方案比较

### 方案 1：独立运行时工作项层（采用）

保留 `ProjectLayout.tabs` 和现有终端状态不变，在每个项目旁增加动态文件会话。统一 Tab 栏把
“配置中的终端 tab”与“运行时打开文件”合并成展示项。

优点：

- 不把动态文件写入 `.yttt/layout.toml`。
- 现有终端命令与状态模型只需适配 active work item，不需要整体重写。
- 项目文件会话可以独立测试。
- 后续仍可演进为更通用的 Pane/Item。

代价：

- 终端 tab 选择与统一 work item 选择需要一个明确同步边界。
- Tab 渲染层需要合并两个来源。

### 方案 2：把 `TabConfig/TabState` 改成终端/文件枚举（不采用）

虽然数据结构表面统一，但会把动态文件混入持久 layout 语义，并迫使现有终端操作、序列化和测试
全面改写。

### 方案 3：重建 Zed 风格通用 Pane/Item workspace（不采用）

最适合未来做文件分屏、diff 和 preview tab，但当前会变成一次 workspace 内核重写，远超本阶段范围。

## 总体架构

### 现有终端模型保持不变

以下模型继续只表达终端工作区：

- `ProjectLayout.tabs`
- `TabConfig`
- `TabState`
- `PaneState`
- terminal split tree 与进程状态

`OpenedProject.selected_tab_id` 继续表示“最近选择的终端 layout tab”。当文件工作项激活时，不清空
这个值。这样从文件切回终端时可以恢复最近终端 tab，终端相关命令也不会被动态文件路径污染。

### ProjectWorkItemSession

UI 会话层按 `ProjectId` 维护一个纯状态 `ProjectWorkItemSession`，职责包括：

```text
open_files: ordered canonical project paths
active_work_item: Terminal(tab_id) | File(canonical_path)
activation_history
file_tree_state
project_panel_visible
project_panel_width
```

命名可在实现时根据现有模块约定微调，但边界必须保持：它是运行时状态，不进入 `ProjectLayout`
序列化。

新项目打开时：

- `active_work_item` 从现有 `selected_tab_id` 初始化。
- `project_panel_visible` 从 `project_panel.default_open` 初始化。
- `project_panel_width` 从持久设置初始化。
- 文件树以项目规范化根目录初始化，但不递归预读所有目录。

切换项目时：

- 只切换可见的 `ProjectWorkItemSession`。
- 原项目的文档、输入内容、脏状态、Tab 顺序、树状态和面板宽度保留在内存。
- 顶部不显示其他项目的工作项。

关闭项目时：

- 先完成脏文件与运行进程的统一关闭决策。
- 只有用户确认后才销毁该项目的文件会话和编辑器实体。

### OpenEditorDocument

每个 `(ProjectId, canonical_path)` 对应一个 UI 层文档对象。它组合：

```text
CodeEditorState
Entity<InputState>
InputState subscription
saved disk fingerprint
load/save state
pending autosave generation
```

纯 `Workspace` 模型不持有 GPUI `Entity`。实体和 subscription 由 editor workspace/runtime 层管理，
避免继续把大量编辑器字段直接堆到已经过大的 `RootView`。

磁盘指纹至少包含：

- 文件是否存在。
- 文件长度与修改时间。
- 已读取内容的运行时 hash。

保存前在后台重新读取并比较指纹，避免仅依赖时间戳导致同长度内容变化漏检。

### UnifiedTabItem

Tab 渲染使用只读展示模型：

```text
Terminal {
    tab_id,
    title,
    process / agent status,
    selected,
}

File {
    canonical_path,
    relative_path,
    basename,
    language / file icon,
    dirty,
    selected,
}
```

顺序规则：

1. 终端 layout tabs 按配置顺序显示。
2. 文件 tabs 按首次打开顺序追加。
3. 文件夹 toggle 固定在可滚动 Tab 容器之后。

同一规范化路径重复打开时只激活已有 Tab，不改变顺序，不创建新的 `InputState`。

关闭当前工作项后，优先激活右侧相邻项；没有右侧项时激活左侧相邻项。文件 Tab 的 tooltip
显示项目相对路径，避免同名文件无法区分。

## 主界面结构

```text
Titlebar
└── Workbench body
    ├── Left opened-project sidebar
    ├── Left resize handle
    ├── Center
    │   ├── Unified tab bar
    │   │   ├── Scrollable terminal + file tabs
    │   │   └── Fixed project-tree toggle
    │   └── Active surface
    │       ├── Terminal split tree, or
    │       └── Project file editor
    ├── Right resize handle, only while tree is visible
    └── Right project file tree, only while visible
```

### 文件夹按钮状态

- 文件树打开：使用 active surface 背景、强调色和 pressed/selected 状态。
- 文件树关闭：使用普通 toolbar icon 状态。
- hover tooltip 随状态显示“显示项目文件”或“隐藏项目文件”。
- 按钮始终位于 Tab 栏最末端，不进入 Tab scroll area。
- setting 只决定新项目会话的初始状态；手动 toggle 决定当前会话状态。

### 文件 Tab 状态

- 前导：语言或通用文件图标。
- 主标签：basename。
- 未保存：脏点。
- 关闭：hover 时显示 close 按钮；脏点与 close 不能造成布局跳动。
- 双击重命名只继续用于终端 tab；文件 tab 不进入重命名流程。
- 文件保存后清除脏点。

## 项目文件树

### 节点模型

每个节点至少保存：

```text
relative_path
display_name
kind: directory | file | symlink
expanded
selected
load_state: unloaded | loading | loaded | error
children
optional git status tone
```

模型保存项目相对路径；实际 I/O 前通过项目根目录解析并验证规范化路径。

### 加载策略

- 项目根目录首次显示时后台读取一级子项。
- 目录第一次展开时后台读取它的直接子项。
- 不在项目打开时递归扫描整个项目。
- refresh 重扫根节点和当前已展开目录。
- 刷新后保留仍存在路径的 expanded/selected 状态。
- 缺失节点从树中移除，但已经打开的文件 Tab 不自动关闭。

### 排序与过滤

- 目录在文件之前。
- 每组内按不区分大小写的 display name 排序，以原名称作为稳定次级排序。
- `show_hidden = false` 时隐藏 basename 以 `.` 开头的条目。
- 本阶段不实现复杂 exclude glob 或完整 `.gitignore` 过滤。
- 符号链接目录作为不可展开叶节点，不递归跟随。
- 符号链接文件只有在规范化目标仍位于项目根目录内时才可打开。

### 交互

- 单击目录：展开或折叠。
- 单击文件：打开或激活持久文件 Tab，并把焦点交给编辑器。
- 当前激活文件在树中显示 selected 状态。
- header 显示当前项目名和 refresh 按钮。
- Git status 可用时为文件显示基础 modified/added/deleted/untracked tone。
- Git status 失败时不显示装饰，不阻塞目录加载。

## 可拖拽 Sidebar

### 共享状态与算法

左项目栏和右文件树共用一个 `ResizableSidebar` 状态/算法边界：

```text
side: Left | Right
current_width
min_width
max_width
collapsed_or_hidden
active_drag_start
```

方向规则：

- 左栏向右拖增大，向左拖减小。
- 右栏向左拖增大，向右拖减小。
- 每次 pointer delta 都 clamp 到允许范围。
- 拖拽 hit area 为 5px，视觉分隔线保持 1px。
- drag active 时使用 `col-resize` 光标并持续更新布局。
- mouse up 后结束 drag，并持久化最后宽度。

默认范围：

- 左栏：160–420px，展开默认 216px，折叠宽度仍为 46px。
- 右栏：200–520px，默认 280px，关闭宽度为 0。

左栏折叠不会覆盖保存的展开宽度。右栏关闭不会把保存宽度改成 0；再次打开恢复之前宽度。

每个已打开项目保留自己的当前右栏宽度。拖拽完成后同时把该值写回全局默认设置，供以后打开的
项目和下次应用启动使用；已经打开的其他项目不被强制改宽。

## 文件打开流程

1. 用户点击文件树节点。
2. 根据所选项目根目录解析相对路径。
3. 规范化路径，并验证目标仍位于项目根目录内。
4. 如果对应文档已经打开，直接激活并聚焦。
5. 后台读取 metadata 和内容。
6. 拒绝超过 10 MiB、包含 NUL 的二进制内容或非 UTF-8 内容。
7. 使用 `EditorLanguageCatalog` 和路径/内容自动解析语言。
8. 根据当前 editor settings 创建 `CodeEditorState` 与 `InputState`。
9. 记录磁盘指纹并建立输入订阅。
10. 追加文件 Tab、设为 active work item，并聚焦编辑器。

任何失败都通过现有 notification/toast 边界报告；失败时不插入空白文档或伪成功 Tab。

## 编辑与保存

### 脏状态

输入订阅把 `InputState` 当前文本同步到 `CodeEditorState`。当前值与 saved baseline 不同即为 dirty。
错误或诊断变化不能误清 dirty；只有成功保存或用户确认 reload 才更新 baseline。

### 手动保存

增加明确的 file save command，并绑定平台主快捷键：

- macOS：`Cmd+S`
- 其他平台：`Ctrl+S`

只有 active file work item 响应该命令。终端 work item 激活时不触发文件写入。

保存流程：

1. 捕获当前文本和文档 generation。
2. 后台读取当前磁盘版本。
3. 若磁盘指纹与打开/最近保存时不同，进入冲突决策。
4. 无冲突时在目标同目录创建临时文件。
5. 写入全部 UTF-8 内容，保留原文件权限，并原子 rename 到目标路径。
6. 只有当前 generation 仍与写入内容相符时，更新 saved baseline 和磁盘指纹。
7. 若用户在后台保存期间继续输入，新输入仍保持 dirty。

### 外部修改冲突

本阶段不使用 watcher。以下时机会检查磁盘版本：

- 文件重新获得焦点。
- project panel refresh。
- 手动或自动保存之前。

行为：

- 文档 clean 且磁盘改变：允许重新加载，并重新解析语言。
- 文档 dirty 且磁盘改变：显示“覆盖磁盘 / 重新加载 / 取消”。
- 文件被外部删除：保留 Tab 与内存内容；保存前询问是否重新创建。
- reload 会明确告知本地未保存内容将被丢弃，不静默执行。

### Autosave

支持：

```text
off
on_focus_change
after_delay
```

- 默认 `off`。
- `on_focus_change` 在文件工作项失去焦点、切换项目或切换工作项时触发。
- `after_delay` 在最后一次输入后的配置延迟触发。
- 延迟任务使用 generation/token 取消过期保存。
- autosave 同样执行磁盘冲突检查。
- 失败或冲突时保留 dirty，并显示通知。

## 关闭保护

### 关闭文件

clean 文件直接关闭。dirty 文件提供：

```text
保存
丢弃
取消
```

保存失败时不关闭 Tab。取消时保持 Tab 和焦点不变。

### 关闭项目和应用

关闭阻塞项统一收集：

- dirty project files。
- 现有 running terminal panes。

项目关闭对话框需要同时呈现两类信息，避免先后弹出互相覆盖的 modal。至少提供：

- 保存全部可保存文件并继续。
- 明确丢弃未保存文件并继续。
- 取消关闭。

运行进程仍遵循现有确认/终止语义。任一保存失败时项目保持打开。

## 设置设计

保留现有 `editor.auto_detect_language`、`editor.default_language` 与 `editor.lsp`，新增：

```toml
[editor]
font_family = ""
font_size = 14.0
line_height = 1.4
tab_size = 4
soft_wrap = false
line_numbers = true
autosave = "off"
autosave_delay_ms = 1000

[project_panel]
default_open = true
show_hidden = false
width = 280.0
project_sidebar_width = 216.0
```

约束和回退：

- 空 `font_family` 表示系统/主题默认等宽字体。
- `font_size` 必须为正值，并限制到合理显示范围。
- `line_height` 至少为 1.0。
- `tab_size` 限制为 1–16。
- `autosave_delay_ms` 必须大于零，并设最小防抖值。
- sidebar 宽度在加载时 clamp 到各自 min/max。
- 非法枚举或数值回退到默认值并产生 `SettingsLoadWarning`。

运行时应用：

- 字体、字号、行高、soft wrap 和 line numbers 立即应用到全部打开编辑器。
- `gpui-component 0.5.1` 没有公开的运行时 tab-size setter，因此 `tab_size` 保存后立即用于新打开
  的文件，已经打开的文件在关闭并重新打开后生效。这是本阶段唯一不实时更新的编辑器显示设置。
- 应用显示设置时不能替换文本、saved baseline、undo/selection 状态；不得为了实时改变
  `tab_size` 而重建 `InputState` 或维护组件库 fork。
- autosave 模式立即影响后续输入与 focus 事件。
- `show_hidden` 变化触发可见树重算/刷新。
- `default_open` 只影响以后创建的项目会话。
- sidebar width 设置作为新会话默认值；当前被拖拽的会话保持显式宽度。

Settings UI 新增独立 Editor 分组或把现有 Languages 分组拆分为 Editor 与 Languages，确保上述核心项
都能通过 UI 修改并持久化。复杂 file type、LSP adapter 和 per-language 覆盖继续通过 TOML 或未来阶段处理。

## 命令与焦点

新增或扩展命令：

```text
file.save
project_panel.toggle
project_panel.refresh
```

现有 `tab.close` 针对 active work item 分派：

- active terminal：沿用现有终端 tab 关闭行为。
- active file：进入文件关闭保护。

`tab.new` 和 pane split/focus/resize 命令仍只操作终端工作项。active file 时不得误改最近终端 tab；
不支持的 editor split 操作可以保持 no-op，并通过命令 enabled 状态避免误触。

打开文件后编辑器获得键盘焦点。点击树和拖拽 sidebar 不得让 terminal input gate 误接收按键。
Settings、dialog、palette 等现有 input owner 优先级保持不变。

## 错误与降级

- 路径规范化失败或越界：拒绝并通知。
- 目录读取失败：节点进入 error 状态并允许 refresh/retry；其他树节点继续工作。
- 文件不存在、无权限或读取失败：不创建 Tab，显示具体路径与错误。
- 超过 10 MiB：提示当前大小限制。
- 二进制或非 UTF-8：提示本阶段不支持。
- 保存临时文件、权限复制或 rename 失败：保留 dirty 与原 Tab。
- Git status 获取失败：只去掉装饰。
- 语言识别或 highlighter 不可用：降级为 plain text，仍允许编辑。
- 外部删除：保留内存内容，保存前确认重建。
- 后台任务完成时项目或文档已关闭：按 project/document generation 丢弃过期结果。

所有用户可见字符串进入现有 i18n 边界。

## 模块边界建议

实现计划应优先形成以下职责，而不是继续扩大 `RootView`：

```text
src/ui/editor/document.rs
  -> OpenEditorDocument、磁盘版本、dirty/save/autosave 状态

src/ui/editor/workspace.rs
  -> 每项目 ProjectWorkItemSession、统一选择与 Tab 合并模型

src/ui/editor/file_io.rs
  -> 受限文件读取、指纹、冲突检测、原子写入

src/ui/project_tree/state.rs
  -> 纯文件树节点、展开/选择/刷新状态

src/ui/project_tree/fs.rs
  -> 目录读取与路径安全边界

src/ui/project_tree/view.rs
  -> 右侧项目树渲染与事件

src/ui/primitives/sidebar.rs
  -> 共享 sidebar style、方向、drag/clamp 算法

src/ui/tabs.rs
  -> 统一 terminal/file tab 展示与固定 project panel toggle
```

实际文件名可以按代码库约定微调，但纯模型、I/O、GPUI entity 和 view 必须保持分层。

## 测试策略

### 纯模型测试

- 目录优先与稳定排序。
- hidden 条目过滤。
- 目录 lazy load、展开、折叠与 refresh 状态保持。
- 路径规范化、项目根越界和 symlink 限制。
- 同路径文件去重。
- 终端与文件 Tab 合并顺序。
- 文件关闭后的右邻/左邻选择。
- 项目切换恢复 active work item、树状态和宽度。
- 左右 sidebar delta 方向、min/max clamp、折叠/隐藏宽度恢复。

### 文件与编辑测试

使用 `tempfile` 覆盖：

- UTF-8 文件读取与自动语言解析。
- 二进制、非 UTF-8 和超限文件拒绝。
- dirty baseline。
- 原子保存和权限保留。
- 保存中继续输入仍保持 dirty。
- 外部修改冲突。
- 外部删除后的重建确认。
- 三种 autosave 模式与过期 generation 取消。
- 保存失败不关闭文件或项目。

### 设置测试

- 默认值。
- TOML 序列化/反序列化 round trip。
- 非法字体数值、tab size、autosave、delay 和宽度回退。
- Settings UI metadata 与 i18n 文案覆盖。
- 设置更新不会改变 editor text 和 saved baseline。

### 集成与 UI 状态测试

- 临时项目：打开树、打开文件、编辑、保存、关闭。
- 重复点击同文件只激活一个 Tab。
- 文件夹按钮与 panel visible 状态一致。
- 切换项目不关闭原项目文档。
- 关闭 dirty file/project 的三分支决策。
- running terminal panes 与 dirty files 的组合关闭阻塞。
- active file 时终端 split 命令不误操作。
- active terminal 时现有行为保持不变。

### 验证命令

```text
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features
```

最后在 macOS dev app 中人工验证：

- 左右 sidebar 拖拽与宽度恢复。
- 文件树 toggle 激活态。
- 多文件 Tab、同名文件 tooltip、脏点与 close hover。
- 手动保存与 autosave。
- 项目切换恢复。
- 外部修改冲突。
- dirty file + running terminal 的项目关闭保护。

## 成功标准

- 用户可以从右侧项目树打开、编辑和保存普通 UTF-8 项目文件。
- 终端与文件共享一条稳定的顶部 Tab 栏，文件夹 toggle 固定在最末端。
- 项目切换不丢文件会话，项目关闭不丢未保存内容。
- 左右 sidebar 都能平滑拖拽并恢复有效宽度。
- 编辑器核心设置可从 Settings 修改、持久化并真实生效。
- 文件 I/O 不阻塞 UI，过期后台结果不会复活已关闭状态。
- 现有终端 layout、split、命令、设置与测试继续通过。
