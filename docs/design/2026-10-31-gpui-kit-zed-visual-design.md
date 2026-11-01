# gpui-kit 迁移与 Zed 视觉对齐设计

- 日期：2026-10-31
- 状态：待评审；尚未开始实现
- 事实基线：yttt `bd8f930`（0.3.18）。`src/ui`、`crates/yttt-ui`、`vendor/gpui-component`、`vendor/gpui-markdown-editor`、`Cargo.toml` 与 `master`（`e6591e0`，0.3.19）无差异
- 实施计划：[2026-10-31-gpui-kit-zed-visual-plan.md](2026-10-31-gpui-kit-zed-visual-plan.md)

## 1. 背景

用户希望 yttt 的按钮、弹层、picker、Markdown、文件树等界面达到 Zed 的视觉效果，并确认使用 `gpui-kit` 作为实现基础。

`gpui-kit` 是 `gpui-component` 改名后的项目：`longbridge/gpui-component` 的 GitHub API 已指向 `longbridge/gpui-kit`，`gpui-component` 现在是其中的 styled 层。所以这项工作是**一次组件库升级**，加**一次 UI 层重建**，不是引入新框架。

本文使用 `codebase-design` 的术语：module、interface、seam、adapter、depth、leverage、locality。

## 2. 已核实的事实

| 主题 | 事实 | 来源 |
|---|---|---|
| GPUI 当前版本 | Zed git rev `1d217ee`，2026-06-12 | `Cargo.toml`，`sources/zed` |
| 上游目标版本 | `gpui-kit`、`gpui-base`、`gpui-component` 0.8.1，`gpui-pre` 0.3.10，均已发布到 crates.io（2026-10-26） | crates.io API |
| 本地 vendored 版本 | 声明 0.5.2；内容最接近上游 `a9a7341c`（2026-07-02） | 按文件内容哈希比对 |
| 本地补丁规模 | 相对该基线约 55 个源文件内容不同，另有 `languages.rs` 和 4 个新增语言目录，合计约 1738 行增删 | `diff -ru` |
| GPUI 差异 | `crates/gpui/src` 有 77 个路径不同，约 23458 行增删；`window.rs` 7119 行，`text_system/line.rs` 1096 行 | `diff -ru` |
| 公开名称 | 从约 1495 个增至 1887 个，移除 10 个；名称级扫描未发现 yttt 真实命中 | 脚本；**不覆盖签名变化** |
| 已知 GPUI 破坏 | `ScrollWheelEvent` 改名 `ScrollEvent`（旧名保留为 deprecated alias），`PlatformInput::ScrollWheel` 变为 `PlatformInput::Scroll`，事件新增 `source` 字段；yttt 与 Markdown 编辑器有 11 处引用 | `gpui-pre` 源码，grep |
| 组件库破坏 | `InputState` 从 0.6 起只支持单行；代码编辑器改用 `EditorState`，多行输入改用 `TextareaState` | 0.6.0 发布说明 |
| yttt 调用面 | `src` 中 48 个文件引用 `gpui_component`；非测试文件里 `input` 在 20 个文件、`Icon` 11 个、`menu` 8 个、`highlighter` 6 个 | 脚本 |
| Root 入口 | 至少 12 处直接构造 `Root`，其中 7 处在 `command_center.rs` | grep |
| 现有 seam | `yttt-ui`（2546 行）已封装 primitives，`clippy.toml` 禁止业务代码直接构造 `Button`、`Input`、`Select` 等 | 源码 |
| 代码编辑器入口 | 集中在 `src/ui/editor/view.rs` 的 `code_editor_input_state`、`styled_code_editor_input`，但 `src/ui/editor` 里 `InputState` 相关引用共 71 处 | grep |
| 上游未覆盖的本地 API | `InputCursorShape`、`selection_anchor/head`、`set_cursor_offset`、`begin/end_history_group`、`move_display_lines`、`move_to_search_match`、`search_panel_is_open`、`flush_search_panel`、`start_grouping_new_version` 在 0.8.1 的 `gpui-base` input 模块里都查不到同名 API | grep |
| 视觉回归能力 | `gpui-pre` 提供 `HeadlessAppContext::capture_screenshot` 和 `Window::render_to_image`，受 `test-support` 控制；具体平台实现未验证 | 源码 |
| `gpui-base` 缺口 | 没有 context menu 和 command palette；这两者只在 `gpui-component` | 源码 |
| `gpui-base` 导出方式 | 组件模块大多私有，从 crate 根导出，比如 `gpui_base::Button`、`gpui_base::TreeState`；`input`、`text` 是公开模块 | `crates/base/src/lib.rs` |
| 风格开关 | `UiStyleId` 默认就是 `Zed`；`Rounded` 是第二个 token 集 | `crates/yttt-ui/src/style.rs` |

**尚未验证：** 没有试编译、没有运行升级后的应用、没有逐行审阅全部补丁。第 7、8 节的结论要在计划的 P1 里验证。

## 3. 目标

1. 把 GPUI 和组件库升级到 `gpui-pre` 0.3.10 与 gpui-kit 0.8.1，保留现有功能。
2. 让 `yttt-ui` 成为业务代码唯一的 UI seam，交互建立在 `gpui-base` 上，外观由 yttt 决定。
3. 在固定的 Zed 版本、主题、字体和缩放下，让按钮、弹层、picker、文件树、Markdown、标题栏、状态栏等界面与 Zed 视觉一致。
4. 用自动化测量守住一致性，而不是靠人工观感。

## 4. 非目标

- 不复制 Zed 的 GPL 代码，也不依赖 `ui`、`picker`、`markdown`、`project_panel`、`theme`、`icons` 等 GPL crate。
- 不重写终端渲染内核，只保证它在升级后不退化。
- 不替换 `gpui-markdown-editor`。
- 不引入 Zed 的 workspace、editor、multi-buffer、collab 架构。
- 不实现 Zed 的 UI density 三档；只对齐默认档。
- 不改变 Agent 侧边栏、Command Center 的产品结构，只应用同一套视觉语言。
- 不引入 `gpui-shell`、`gpui-fps`、WebAssembly、语音等能力。
- 本项目不开启 GPUI Fast，视觉验收完成后再单独评估。

## 5. 关键决策

### D1：先升级，再做视觉

先完成一次“外观不变”的升级（M1），再开始视觉重建（M2）。

原因：`gpui-base` 只存在于 0.6 之后；在旧版上做视觉，之后还要重做一遍。两件事混在一起时，回归也无法归因。

代价：M1 必须大范围一次切换。Cargo 不允许两份 `gpui` 的类型混用，所以无法用开关灰度。

### D2：依赖走 crates.io，补丁走 `[patch.crates-io]`

目标形态：

```toml
[workspace.dependencies]
gpui = { package = "gpui-pre", version = "=0.3.10" }
gpui_macros = { package = "gpui-pre-macros", version = "=0.3.10" }
gpui_platform = { package = "gpui-pre-platform", version = "=0.3.10" }
reqwest_client = { package = "gpui-pre-reqwest-client", version = "=0.3.10" }
gpui-base = "=0.8.1"
gpui-component = "=0.8.1"
gpui-kit-assets = "=0.8.1"

[patch.crates-io]
gpui-base = { path = "vendor/gpui-base" }
gpui-component = { path = "vendor/gpui-component" }
```

- 用 `package` 别名保留现有的 `use gpui::...`，避免无意义的大范围改写。
- 不使用 `gpui-kit` facade crate：它能对齐版本，但要改掉全部 import。由 `Cargo.lock` 和单一 GPUI 检查保证版本对齐（见第 12.2 节）。
- 所有版本用 `=` 精确锁定。上游明确说明任何快照都可能改变 API，并已因此出过事故。
- `vendor/gpui-base`、`vendor/gpui-component` 从发布包解开，manifest 不依赖上游 workspace 继承。
- 每个补丁都记录在 `vendor/PATCHES.md`：来源、分类、删除条件。
- 补丁清零的 crate 直接删除 `[patch.crates-io]` 条目和目录，业务代码零改动。

放弃的方案：

| 方案 | 放弃原因 |
|---|---|
| 不打补丁，直接用 crates.io | 编辑器和 vim 依赖上游没有的 API，第一天就编译不过 |
| 继续用路径依赖 vendored 全量副本 | 补丁和上游内容混在一起，无法度量和缩减 |
| 建 GitHub fork，用 git 依赖 | 增加一个外部仓库，当前收益不足；上游合并补丁后可再评估 |

### D3：`yttt-ui` 是唯一的 seam，`gpui-base` 是实现细节

业务代码（`src/ui/**`）只面向 `yttt-ui` 的 interface。`gpui-base` 不直接暴露给业务代码，否则外观决策会重新散落到调用方。

不为 `gpui-base` 再包一层 trait。这里只有一个 adapter，抽象是假想的 seam。`yttt-ui` 提供的是有产品语义的函数，比如 `yttt_button(...)`；需要变化时直接修改实现。

### D4：真正有变化的 seam 是 `UiStyle`

`Zed` 和 `Rounded` 是同一 interface 的两个 adapter，这个 seam 是真实的，保留。`Rounded` 只保证能用，不进入 Zed 像素门禁。

所有 primitives 只读取 `UiStyle` 和 `WorkbenchTheme`，不写字面量尺寸或颜色。

### D5：组件来源分三类

| 来源 | 适用对象 | 规则 |
|---|---|---|
| `gpui-base` + yttt 外观 | Button、Tooltip、Popover、Select、Combobox、Dialog、Tabs、Switch、Radio、Tree、Input、Scrollbar | 默认选项 |
| yttt 自建 | Context menu、Picker、Indent guides、Keybinding hint、Status item | `gpui-base` 没有或不匹配 Zed 行为 |
| 继续用现有实现 | 终端渲染、`gpui-markdown-editor`、代码高亮、Notification 过渡期实现 | 不在视觉重建范围，只做主题对齐 |

### D6：编辑器保留本地补丁，优先推动上游

`InputCursorShape`、撤销分组、显示行移动等 API 是 vim 模式和编辑器命令依赖的通用能力，且没有上游替代。

M1 先把它们重放到 `vendor/gpui-base`。之后尝试向上游提交。提交 PR 是对外操作，需要用户另行批准；未批准时补丁继续留在 `vendor`。

### D7：对齐范围的固定方式

“和 Zed 一样”的含义是：在基线配置下与**某个固定 Zed 正式版**一致。

- 基线版本在 P0 开始时选定，取当时的 Zed stable，写入 `tests/visual/zed/manifest.toml`。
- `sources/zed` 切到同一 tag，只读参考。
- 之后对齐 Zed 新版本是独立任务，不属于本项目。
- 基线主题 One Dark 和 One Light，UI 字体 IBM Plex Sans，代码字体 Lilex，字号显式写入两边配置。
- 终端默认字体和字号不因本项目改变，避免影响现有用户习惯和 Nerd Font 图标。Lilex 主字体加 Nerd Font 回退作为 P8 的独立验证项。

### D8：默认行为以 Zed 为准

升级引入的默认行为凡与 Zed 不同，一律在 `yttt-ui` 关掉，见第 11 节。

### D9：Rems 优先，遵守 UI 字号缩放

新 token 用 rem 表达，像素值只用于 hairline 和阴影。这样 `ui_font_size` 改变时密度比例和 Zed 一致。

### D10：删除测试决定是否保留 `gpui-component`

M2 结束时，`src/` 不再出现 `gpui_component`，由编译器而不是 lint 保证：根 crate 的 dependencies 移除它，只有 `yttt-ui` 可以依赖。

`gpui-component` 能否从 `yttt-ui` 彻底移除，等 P9 按剩余用量决定，不预先承诺。

## 6. 目标架构

```text
src/ui/**  业务 view：workbench、settings、project_tree、palette、editor
   │  只依赖 yttt-ui 的 interface
   ▼
crates/yttt-ui
   ├─ style      UiStyle：token 集，Zed / Rounded 两个 adapter
   ├─ theme      WorkbenchTheme：语义颜色
   ├─ window     窗口与 Root 的唯一入口，YtttRootPlugin
   ├─ surface    surface / elevation 组合
   ├─ primitives Button、Input、Select、Tooltip、Tabs ...
   ├─ menu       MenuEntry 数据模型与 context menu
   ├─ picker     PickerDelegate 与 Picker
   └─ tree       tree row、indent guides
   │
   ├────────────► gpui-component（过渡期；范围逐步缩小）
   ▼
gpui-base（行为、状态、定位、焦点、虚拟列表、input engine）
   ▼
gpui-pre（默认后端，Zed GPUI 快照）

crates/yttt-terminal、vendor/gpui-markdown-editor  直接依赖 gpui-pre
```

`gpui-base` 的文档也规定：base 负责行为和实现行为所需的几何计算，颜色、字体、控件密度、边框、圆角、图标和最终组合由表现层负责。这与 D3 一致。

## 7. 本地补丁处置

补丁按去向分类。最终处置在 P1 逐文件确认，写入 `vendor/PATCHES.md`。

| 类别 | 内容 | M1 处置 | 最终去向 |
|---|---|---|---|
| A 编辑器能力 | 第 2 节列出的 input API；`input/state.rs`（+190/−6）、`input/search.rs`（+61/−2）、`input/movement.rs`、`history.rs`；`input/input.rs` 的 `flush_search_panel`；`tree.rs` 的 `focus_handle` | 重放到 `vendor/gpui-base`，按新的 Input/Editor 分层调整 | 上游合并后删除 |
| B 编辑器绘制 | `input/element.rs`（+248/−226），gutter 裁剪等 | 先对照 0.8.1 重新实现，不机械重放 | 上游合并或继续留在 `vendor` |
| C 呈现覆盖 | `PopupMenuStyle`、`MenuItemAppearance`、`TooltipAppearance`、`always_show_close_button`、`hover_matches_selected`、placeholder 颜色 | 重放到 `vendor/gpui-component`，保证 M1 外观不变 | M2 被自有 primitive 取代后删除 |
| D 语言支持 | `highlighter/languages.rs`（+120/−4），dockerfile、hcl、tsx、vue | tsx 改用上游 feature；其余先重放 | 优先通过 `LanguageRegistry` 在 yttt 侧注册，能做到就删除补丁 |
| E 平台修复 | Windows 标题栏拖动（`d3ad066`）等 | 对照 `docs/windows-titlebar-verification.md` 验证，需要时重放 | 上游已修复则删除 |
| F 零散小改 | dock、dialog、breadcrumb、clipboard 等 2 到 40 行的改动 | 默认不重放；编译或测试失败时再逐项恢复 | 不保留无证据的改动 |

## 8. GPUI 与组件库迁移面

| 区域 | 改动 | 风险 |
|---|---|---|
| Cargo | 根 crate、`yttt-ui`、`yttt-terminal`、`gpui-markdown-editor`、`gpui_platform`、`reqwest_client` 同时切换 | 中 |
| 滚动事件 | 11 处改用 `ScrollEvent` / `PlatformInput::Scroll`，字面量补 `source` | 低 |
| `Root` | `Root` 已下沉到 `gpui-base`；至少 12 处调用统一到 `yttt_ui::window` | 中 |
| 输入框 | 单行继续用 `InputState`，核对 API 变化 | 中 |
| 代码编辑器 | `Entity<InputState>` 改为 `Entity<EditorState>`，波及 `document.rs`、`vim.rs`、`layout_editor_controller.rs`、`render.rs` | **高** |
| 主题 | `ThemeConfig`、schema 变化；`ThemeRuntime::to_gpui_component_theme_config` 要重新映射 | 中 |
| 高亮 | `HighlightTheme`、`LanguageRegistry` 路径和 API 核对；tree-sitter 版本对齐 | 中 |
| 终端 | `line.rs`、`window.rs` 改动大，文字度量和性能都可能变化 | **高** |
| IME | 上游有改进，但 `yttt-terminal` 自己实现输入，不会自动受益 | 中 |
| Windows、Linux | 平台 crate 差异未评估 | 中 |

M1 的停止条件写在计划的 P1 退出门禁中。

## 9. 视觉规范

### 9.1 Token 差距

以下 Zed 值取自 1.12.0，只用于建立差距清单。最终值以 D7 选定版本的渲染测量为准。

| 项目 | Zed 参考 | yttt `UiStyle::zed()` | 判断 |
|---|---|---|---|
| 默认按钮高度 | 22px | `rems(1.375)`，22px | 已一致 |
| 紧凑按钮高度 | 18px | 未见独立 token | 待补 |
| 中、大按钮高度 | 28px、32px | 未见独立 token | 待补 |
| Tab 栏高度 | 32px | `rems(2.0)` | 已一致 |
| Tab 圆角 | 无 | `px(0.0)` | 已一致 |
| Modal 宽度 | 34rem | `px(544.0)` | 数值一致，改用 rem |
| 文字阶梯 | 10 / 12 / 14 / 16px | 未见统一 token | 待核对 |
| Tooltip 显示延迟 | 500ms | 待测 | 待核对 |
| 浮层阴影 | 3 级 elevation，多层阴影 | 不统一：tooltip 已是双层阴影；panel 用单层 `shadow_lg`；`component.shadow` 为 false，影响通知和组件主题 | **需要统一** |
| 列表行 | Dense，缩进步长 12px | palette 行 48px，sidebar 行 28px | 待测 |
| 文件树 | 默认宽 240，缩进 20，comfortable 间距 | 待测 | 待核对 |
| macOS 标题栏 | `max(1.75rem, 34px)` | 待测 | 待核对 |
| 状态栏 | padding 4px，gap 8px | 待测 | 待核对 |

### 9.2 新增 token

在 `UiStyle` 中增加：

```rust
pub struct UiElevationScale {
    pub surface: UiElevation,
    pub elevated: UiElevation,
    pub modal: UiElevation,
}

pub struct UiElevation {
    pub background: UiSurfaceRole,
    pub border: UiBorderRole,
    pub radius: Pixels,
    pub shadow: SmallVec<[BoxShadow; 4]>,
}

pub struct UiTextScale {
    pub xsmall: Rems,
    pub small: Rems,
    pub default: Rems,
    pub large: Rems,
}

pub struct UiMotion {
    pub tooltip_show_delay: Duration,
    pub tooltip_grace: Duration,
}
```

`UiStyle` 目前是 `Copy`。阴影使用定长数组或静态切片，保持 `Copy`，避免波及调用方；上面的 `SmallVec` 只表达语义，实现时选定。

字段名是草案。原则是 token 表达角色，不表达组件。组件专属尺寸（比如 `UiPaletteMetrics`）只保留确实无法由通用 token 推出的部分。

### 9.3 字体与图标

- UI 字体沿用 IBM Plex Sans；增加 Lilex 作为编辑器和基线字体。两者都使用 OFL 许可，打包时保留许可文件。
- 优先使用 `gpui-kit-assets` 或项目已有的 Lucide 图标。
- Zed 的图标只有在逐项确认许可后才能引入。`assets/icons/LICENSES` 开头是 Lucide ISC，但没有确认覆盖全部图标。其余图标由 yttt 自绘。
- 不使用 Zed 的 logo 或商标。

## 10. 模块设计

下列 Rust 片段是 interface 草案，不是最终签名。

### 10.1 `window`：窗口入口

问题：`Root` 构造分散在至少 12 处，rem、窗口边框、背景、overlay 规则都由调用方自行保证。

```rust
pub fn open_yttt_window<V: Render + 'static>(
    options: YtttWindowOptions,
    cx: &mut App,
    build: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) -> Result<WindowHandle<Root>>;

pub struct YtttRootPlugin { /* style、theme、material */ }
impl RootPlugin for YtttRootPlugin { /* prepare / style / decorate */ }
```

`prepare` 同步 rem，`style` 应用根背景，`decorate` 处理客户端窗口边框。接入后，调用方不必知道这些规则。

Interface 要点：必须已调用 `yttt_ui::init`；调用方不再自己包 `Root`。

### 10.2 `surface`：浮层外观

```rust
pub trait YtttSurface: Styled + Sized {
    fn yttt_surface(self, level: UiElevationLevel, style: UiStyle, theme: WorkbenchTheme) -> Self;
}
```

Tooltip、menu、picker、popover、dialog 共用，修改一次全部生效。

### 10.3 `primitives`

- **Button / IconButton**：改到 `gpui_base::Button`，由 `yttt_button_style` 决定外观。`gpui_base::Button` 没有默认尺寸和背景，所以外观完全来自 `UiStyle`。增加 size：Compact、Default、Medium、Large。
- **Tooltip**：用 `TooltipDefaults` 安装延迟，内容含文字和 keybinding，外观用 elevation 2。
- **Input**：单行继续包装 `InputState`，`InputPresentation` 决定外观。对外保留 `yttt_input`、`yttt_borderless_input`。
- **Select / Combobox**：基于 `gpui_base::Select`、`gpui_base::Combobox`，列表复用 `picker` 的 row 外观。
- **Tabs**：基于 `gpui_base::Tabs`，边框和 selected 状态按 Zed 表现。
- **Scrollbar**：基于 `gpui-base` 的 scrollbar 导出，编辑器用 `set_scrollbar_renderer` 共享同一外观。
- **Switch、Radio、Checkbox、Dialog**：基于 `gpui-base` 同名模块，保留现有函数名。

`yttt_*` 函数名尽量不变，这样 M2 大部分替换不波及业务文件。

### 10.4 `menu`：context menu

问题：现有菜单依赖 `popup_menu.rs` 的 +252 行补丁；`gpui-base` 没有菜单。

```rust
pub enum MenuEntry {
    Action(MenuAction),
    Submenu(MenuSubmenu),
    Header(SharedString),
    Separator,
}

pub struct MenuAction { /* label, icon, action 或 handler, disabled, checked */ }

pub fn yttt_context_menu(
    id: impl Into<ElementId>,
    trigger: impl IntoElement,
    entries: impl Fn(&mut Window, &mut App) -> Vec<MenuEntry> + 'static,
) -> impl IntoElement;

pub fn yttt_dropdown_menu(/* 同上 */) -> impl IntoElement;
```

实现基于 `gpui_base::Popup`、`Positioner`、`FocusTrapElement`，内部负责：键盘导航、Esc 与点击外部关闭、窗口边缘避让、子菜单打开、从父项移向子菜单时的安全区、keybinding 显示。

调用方只描述数据，所以能用纯数据测试。迁移分两步：P2 先让 interface 落在现有 `gpui_component::menu` 上，P4 替换实现，调用方不再改动。

### 10.5 `picker`

把 `src/ui/palette/picker` 下沉到 `yttt-ui::picker`，统一 command palette、文件搜索、SSH 项目选择、字体选择和带搜索的 select。

```rust
pub trait PickerDelegate: 'static {
    type Item;
    fn items(&self) -> &[Self::Item];
    fn update_query(&mut self, query: &str, cx: &mut Context<Picker<Self>>) -> Task<()>;
    fn selected_index(&self) -> usize;
    fn set_selected_index(&mut self, index: usize, cx: &mut Context<Picker<Self>>);
    fn confirm(&mut self, secondary: bool, window: &mut Window, cx: &mut Context<Picker<Self>>);
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Picker<Self>>);
    fn render_item(&self, index: usize, state: PickerRowState, cx: &App) -> AnyElement;
}
```

接口从现有 `delegate.rs` 演进，不新造。字段最终以现有实现和 `ssh_project_picker.rs` 的实际需求为准。

`Picker` 内部负责查询输入、虚拟列表、选中保持、匹配高亮、空状态、footer、焦点和 `KeyContext`。列表用 `gpui_base::v_virtual_list`，不使用会回弹的 styled 列表。

`ssh_project_picker.rs` 有 2403 行。P5 先迁移 palette，再判断它能否作为 delegate 接入。不能时保留专用 view，只共享 row 和 surface。

### 10.6 文件树

`project_tree` 的业务状态（懒加载、Git 状态、剪切粘贴、重命名）留在 `src/ui/project_tree`。

- 行为：改用 `gpui_base::TreeState`，它负责展开、选中、键盘和虚拟化，调用方负责渲染 `TreeEntry`。
- 外观：`yttt_ui::tree::yttt_tree_row`，统一缩进、图标、Git 颜色、hover 和选中。
- 新增：Indent guides；Sticky 目录头（独立任务，可晚于其他项合入）。
- 接入新的 tree 右键能力后，移除自行拼接的 context menu 逻辑。

Interface 要点：`TreeItem` 的 id 必须在刷新间稳定，否则展开和选中状态会丢失。

### 10.7 代码编辑器

`src/ui/editor/view.rs` 的两个函数继续作为 seam，返回类型改为 `Entity<EditorState>`。vim 通过 A 类补丁 API 驱动编辑器。

新增外观：用 `set_scrollbar_renderer` 接入统一滚动条；gutter、当前行、缩进线、搜索条颜色来自 `editor` theme。

### 10.8 Markdown

| 场景 | 实现 |
|---|---|
| 编辑 | 继续用 `gpui-markdown-editor` |
| 只读预览 | `src/ui/editor/preview.rs` 按 P7 的结果决定是否改用 `gpui_base::text` |

对齐内容：`to_markdown_editor_theme` 补全 token；标题比例、段落间距、列表缩进、代码块、表格、引用块在 `vendor/gpui-markdown-editor` 的 theme 配置层调整。若主题配置表达不了，再修改 vendor。

Mermaid 等 Zed 新增渲染能力不在本项目范围。

### 10.9 Shell

- 标题栏：高度、padding、项目与分支分隔样式对齐。
- 状态栏：按钮尺寸、分组、分隔线对齐。yttt 的底栏语义保留。
- 面板与分割线：1px 分割线、激活态颜色对齐。
- Tab：边框表达选中态、关闭按钮显隐对齐。
- Settings 页和 Command Center：只替换 primitive，不重排布局。

## 11. 需要主动抑制的上游默认行为

| 上游行为 | 引入版本 | 处理 |
|---|---|---|
| 滚动回弹在 macOS 默认开启 | 0.8 | `List`、`Tree`、`Select`、`Command`、`Scrollable` 有 `scroll_bounce` 开关，过渡期关闭；`Sidebar`、settings page、`TextView` 没有开关，不使用或自行包装。`gpui_base::Tree` 未见该开关，是否回弹由 P1 验证。对照的 Zed 1.12 源码在 `gpui` 和 `ui` 中未发现回弹实现 |
| 嵌套滚动由最内层区域独占手势 | 0.8 | 核对设置页、文件树和编辑器内的滚动体验，与基线 Zed 比较 |
| Dialog 入场动画可配置 | 0.7.3 | 配置为与基线 Zed 一致 |
| Tabs、Tooltip 动画 | 0.6 | 按基线 Zed 配置；Zed 没有的动画不使用 |
| Chart 首次绘制动画 | 0.7.1 | yttt 未使用 chart；若引入则关闭 |
| 文本输入滚动不再外溢给页面 | 0.8 | 与基线 Zed 对比后决定，默认接受 |

## 12. 验证体系

### 12.1 四层门禁

| 层 | 内容 | 判定 | 运行时机 |
|---|---|---|---|
| L0 工程 | fmt、clippy、`cargo test`、CI 三平台 | 全部通过 | 每次提交 |
| L1 指标 | 组件尺寸、间距、圆角、token 数值 | 与 `metrics.toml` 完全一致 | 每次提交 |
| L2 自回归 | yttt 组件展示页前后截图对比 | 只允许声明过的变化 | 每个 PR |
| L3 Zed 一致性 | 与基线 Zed 截图的像素对比 | 不超过校准后的阈值或 `exceptions.toml` | 每个视觉 PR，发布前全量 |

行为由 `gpui-base` 的 test-support 风格集成测试覆盖：真实窗口、键盘和指针输入、断言焦点和状态。

### 12.2 组件展示页

`src/bin/yttt-ui-gallery.rs`，`required-features = ["ui-gallery"]`，不进入发布包。

- 固定窗口尺寸、主题、字体、缩放。
- 每个组件的 default、hover、active、focus、selected、disabled 状态并排展示。hover、active 用强制状态参数，不依赖鼠标。
- 关闭光标闪烁，所有弹层可直接展开。
- 可导出 JSON，记录各组件的布局 bounds，供 L1 使用。

单一 GPUI 检查：`cargo tree -d` 中不允许出现两份 gpui，CI 增加检查。

### 12.3 截图方式

| 用途 | 方法 | 说明 |
|---|---|---|
| L2 | `capture_screenshot` / `render_to_image` | 无需显示窗口；只在 M1 后可用，平台支持待验证。不可用时退回 L3 的方式 |
| L3 | macOS `screencapture -o -l <window-id>` | Zed 和 yttt 用**同一条链路、同一台机器、同一缩放** |

截图不放进日常 CI 门禁，直到确认其在 CI 机器上稳定。先作为本地门禁运行。

### 12.4 L3 判定

`scripts/ui_visual_diff.py` 按组件区域比较两张图：

- 统计通道差超过容差的像素比例。
- 先对同一应用连续截图两次，得到噪声下限。
- 初始建议：通道差大于 16/255 的像素不超过区域的 1%。这是待校准的估计值，P0 校准后写入 `manifest.toml`。
- 无法严格一致的区域写入 `exceptions.toml`，必须填写原因和审批人。

### 12.5 手工回归清单

- 中文拼音输入：终端、代码编辑器、Markdown 编辑器、单行输入框，含候选框位置。
- vim：模式切换、光标形状、撤销粒度、搜索。
- Windows 标题栏：按 `docs/windows-titlebar-verification.md` 执行。
- 透明窗口材质、缩放切换、外接显示器。
- 终端：`scripts/run-terminal-perf.sh`，与 P0 基线比较。

## 13. 许可证规则

| 来源 | 许可证 | 规则 |
|---|---|---|
| `gpui-pre`、`gpui-base`、`gpui-component`、`gpui-kit-assets` | Apache-2.0 | 可使用和修改；保留许可和版权声明，修改的文件标明改动 |
| `gpui-kit` 文档正文 | CC BY 4.0 / 见 `LICENSE-DOCS.md` | 只参考，不复制正文 |
| Zed `ui`、`picker`、`markdown`、`project_panel`、`theme`、`icons` 等 | GPL-3.0-or-later | 不复制、不翻译、不依赖 |
| IBM Plex Sans、Lilex | OFL | 可打包，保留许可 |
| Zed 图标、内置主题 | 需逐项确认 | 确认前不打包 |

实施规则：

1. 视觉依据是 Zed 的实际渲染结果、公开文档和用户可观察行为。
2. 不打开 Zed 组件源码来逐段翻译。需要了解结构时，只记录行为和结论。
3. token 数值以渲染测量结果为准。第 9.1 节的参考值只用于列差距。
4. 这些规则是工程约束，不是法律意见。

仓库根目录目前没有 LICENSE，仓库是公开的。这不影响本设计的技术方案，但属于待决定事项。

## 14. 风险

| 风险 | 影响 | 缓解 |
|---|---|---|
| 迁移期间 `master` 继续改 UI | 大量冲突 | 迁移分支定期合并 `master`；P1 合入窗口内暂缓大型 UI 改动 |
| 上游每周发布且可能破坏 | 维护成本上升 | 精确锁定；升级作为独立任务；不追新 |
| 终端性能或文字度量退化 | 核心体验受损 | P0 建基线，P1 量化门禁，不达标不合入 |
| 编辑器补丁无法干净重放 | M1 延期 | P1.0 先做验证；必要时保留过渡实现 |
| `gpui-pre` 快照和基线 Zed 的渲染有差异 | L3 有无法消除的差值 | 选用发布时间最接近的 Zed 正式版；记录到例外清单 |
| vendored 代码增长 | 仓库变大，review 困难 | `vendor` 排除在 workspace 外；用 `PATCHES.md` 管理差异 |
| 其他平台无人验证 | Windows、Linux 回归 | CI 编译加手工清单 |
| 像素门禁不稳定 | 噪声阻塞开发 | 校准阈值；只在同机比较 |
| 过度追求像素一致 | 项目不收敛 | 用 `exceptions.toml` 管理；业务独有界面不进入 L3 |

## 15. 需要用户决定

1. **根目录 LICENSE：** 公开仓库目前没有根许可证。不阻塞实施，但应在下一次发布前决定。
2. **向 gpui-kit 上游提交编辑器 API 补丁：** 对外操作，需要批准。未批准时补丁留在 `vendor`，方案仍然成立。

## 16. 与既有文档的关系

| 文档 | 关系 |
|---|---|
| `docs/superpowers/specs/2026-07-08-zed-like-ui-refresh-design.md` | 延续其“Zed + yttt 产品形态”方向 |
| `docs/superpowers/specs/2026-07-09-zed-reference-interaction-ui-design.md` | 延续 `yttt-ui` 适配层思路。其“保持底层继续使用 `gpui-component`”的约束由本文 D3、D5 更新 |
| `docs/superpowers/specs/2026-07-11-standalone-gpui-editor-components-design.md` | 该方案的 `gpui-editor` 未进入当前依赖。本文采用 `gpui-base` 的 `EditorState`，不在本项目内实施独立 GPL 编辑器 |
| `docs/windows-titlebar-verification.md` | 作为 Windows 回归清单 |
