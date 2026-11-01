# gpui-kit 迁移与 Zed 视觉对齐实施计划

- 日期：2026-10-31
- 设计文档：[2026-10-31-gpui-kit-zed-visual-design.md](2026-10-31-gpui-kit-zed-visual-design.md)
- 状态：M1 已合入，P2 待合入（PR #2），P3 进行中；P0 剩余项见下方进度

## 0. 执行规则

- 每个阶段在 `.worktrees/` 下使用独立 worktree，从最新 `master` 开始，不在主工作区实施。本计划文档直接写入主工作区是用户针对这次文档任务的明确指示，不适用于后续实施。
- 按 `AGENTS.md` 内联执行，不把计划执行委派给 subagent。
- 删除 `.gitignore` 条目前先询问用户。
- 每个任务单独提交；每个阶段单独 PR。
- 视觉变更 PR 附 L2、L3 报告；纯迁移 PR 附“无视觉变化”的 L2 报告。
- 对外操作（向上游提 PR、发布版本）需要用户另行批准。
- 工作量只标相对大小：S、M、L、XL。

通用验证命令，以下称 **L0**：

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --locked -p yttt --tests --no-fail-fast
cargo test --locked -p yttt-client-core --test host_roundtrip --no-fail-fast
```

`validation.yml` 的 clippy 按 package 列出。实施前对照 CI 的实际命令，确保本地不比 CI 宽松。

## 1. 阶段总览

```text
P0 基线与门禁
 │
P1 升级 GPUI 与 gpui-kit（M1，外观不变）
 │
P2 收拢 seam 与 token
 ├─ P3 基础控件
 │   └─ P4 浮层与菜单
 │       ├─ P5 Picker 与 Select
 │       └─ P6 文件树
 ├─ P7 编辑器与 Markdown（依赖 P3）
 └─ P8 Shell 与业务页面（依赖 P3、P4）
      │
P9 全量验收与收尾（M2）
```

| 阶段 | 目标 | 大小 | 可否中止后保留成果 |
|---|---|---|---|
| P0 | 基线、展示页、对比工具 | M | 可以，工具独立有用 |
| P1 | 依赖升级，外观不变 | XL | 不合入即无影响 |
| P2 | 统一入口和 token | L | 可以 |
| P3 | 按钮、输入、tooltip 等 | L | 可以 |
| P4 | 弹层、菜单 | L | 可以 |
| P5 | Picker、Select | M | 可以 |
| P6 | 文件树 | M | 可以 |
| P7 | 编辑器、Markdown | M | 可以 |
| P8 | 标题栏、状态栏、设置页等 | M | 可以 |
| P9 | 收尾、发布准备 | S | 不适用 |

P3 之后的 PR 都能单独发布，界面只会逐步更接近 Zed。

## 2. P0：基线与门禁

目标：在改依赖之前冻结可对比的现状。不改产品行为。

进度（2026-11-01）：0.3、0.4 已完成；0.1、0.2 已完成但有偏差；0.5、0.6 等人工步骤。

- 0.1：基线为 Zed 1.26.2。`sources/zed` 有本地改动，保持原状，未切换 tag。
- 0.2：每组 3 次、无 trace 的开发基线，见 `baselines/terminal-perf-a2160c8.md`。
- 0.4：hover、active 在展示页内用样式数据强制，没有给 primitives 加参数；focus 状态尚未展示。
- 0.4：截图用离屏 `render_to_image`，macOS 上在当前 GPUI 版本已可用，无需屏幕录制权限。
- 0.5：yttt 和 Zed 的截图噪声都测为 0，写入 `manifest.toml`。噪声不能决定跨应用阈值，所以阈值保留初始估计，在 P3 首个组件对比时校准。按组件的 Zed 基线截图尚未采集。
- 0.6：清单已写，需要真人跑一遍。

### 任务 0.1：选定 Zed 基线（S）

- [ ] 确认当时的 Zed stable 版本和发布日期。
- [ ] 将 `sources/zed` 切到对应 tag。它被 `.gitignore` 忽略，不产生仓库变更。
- [ ] 创建 `tests/visual/zed/manifest.toml`，记录 Zed 版本、macOS 版本、机器型号、缩放、窗口尺寸、主题、字体和字号。
- [ ] 创建 `tests/visual/zed/settings.json`，保存基线 Zed 配置。

退出：另一个人按 manifest 能重建同样的 Zed 环境。

### 任务 0.2：终端性能基线（S）

- [ ] 用 `scripts/run-terminal-perf.sh` 在固定机器上运行至少 3 次。
- [ ] 用 `scripts/summarize-terminal-perf.py` 生成摘要。
- [ ] 写入 `docs/design/baselines/terminal-perf-<commit>.md`。

退出：有带 commit、机器和负载信息的基线数据。

### 任务 0.3：视觉差异工具（M）

新增 `scripts/ui_visual_diff.py`、`scripts/test_ui_visual_diff.py`。

- [ ] 先写测试：相同图、单像素差异、区域裁剪、尺寸不一致、例外清单。
- [ ] 实现按区域比较，输出 JSON 和差异图。
- [ ] 支持 `tests/visual/zed/regions.toml`、`exceptions.toml`。
- [ ] 例外必须有 `reason` 和 `approved_by`，否则工具报错。
- [ ] 将测试加入 `validation.yml`。

退出：`python3 scripts/test_ui_visual_diff.py` 通过。

### 任务 0.4：组件展示页（M）

新增 `src/bin/yttt-ui-gallery.rs`，feature `ui-gallery`。

- [ ] 用当前 primitives 展示 button、icon button、input、select、switch、radio、tabs、tooltip、menu、dialog、notification、列表行、树行。
- [ ] 为 primitives 增加强制交互状态参数，仅 gallery 使用。
- [ ] 固定主题、字体、尺寸，关闭闪烁。
- [ ] 增加 `--dump-layout <path>`，导出组件 bounds。
- [ ] 确认发布脚本不打包该二进制。

退出：`cargo run --features ui-gallery --bin yttt-ui-gallery` 可重复渲染，两次截图一致。

### 任务 0.5：采集基线（M）

新增 `scripts/capture-ui-gallery.sh`。

- [ ] 采集 yttt 现状截图，作为 M1 的 L2 基线。
- [ ] 按组件采集 Zed 基线，补齐 `docs/zed-screenshots` 没覆盖的状态。
- [ ] 同一应用连续截图两次，校准噪声。
- [ ] 把阈值写入 `manifest.toml`，替换设计文档里的 16/255 和 1% 初始估计。

退出：有校准后的阈值，文档记录噪声测量结果。

### 任务 0.6：手工回归清单（S）

新增 `docs/design/baselines/ui-regression-checklist.md`。

- [ ] 收录设计文档第 12.5 节内容。
- [ ] 在当前版本上跑一遍，记录已有问题，避免之后误判为回归。

**P0 退出门禁：** L0 通过；展示页可重复；终端基线已记录；阈值已校准。

## 3. P1：升级 GPUI 与 gpui-kit（M1）

目标：外观不变，功能不退化。整个阶段在一个长期分支上完成，合入前不改视觉。

### 任务 1.0：可行性验证（L，限时）

分支 `spike/gpui-kit-0.8.1`，代码不要求合入。

- [ ] 按设计 D2 切换依赖，用 `cargo tree -d` 确认只有一份 gpui。
- [ ] 让 `yttt-terminal` 编译运行，跑 `scripts/run-terminal-perf.sh`，并验证中文 IME。
- [ ] 用 `EditorState` 打开文件，验证编辑、高亮、滚动。
- [ ] 重放 `InputCursorShape` 和撤销分组，验证 vim 基本流程。
- [ ] 记录 API 破坏清单、补丁工作量和性能结果，写入 `docs/design/baselines/gpui-kit-spike.md`。

**继续条件（全部满足）：**

- 终端性能在 P0 基线的 10% 以内，或差异原因明确且可修复。
- 中文 IME 可用。
- vim 依赖的 API 可在 `gpui-base` 中实现。

不满足时停止 P1，向用户报告，并讨论退回“在现有 vendored 组件上改样式”的路线。

结论（2026-11-01）：继续。Host 的帧时间和输入延迟无退化；`burst` 吞吐低约 15%，超出 10%，用户已确认接受。其中约一半来自 `gpui-pre` 强制要求的 Rust 1.98。此后的基线改为“用 Rust 1.98 编译的旧版本”，差异约 -6%。P1 合入前再 profile 一次 `burst`。详见 `baselines/gpui-kit-spike.md`。

### 任务 1.1：建立 vendor 结构（M）

已完成。两个 crate 的 vendor 目录、`[workspace.dependencies]`、`[patch.crates-io]`、`vendor/PATCHES.md`、修改标注和 CI 单一 GPUI 检查都已就绪。`gpui-markdown-editor` 不在 workspace 内，继续写明版本。

- [ ] 从 crates.io 的 0.8.1 发布包解开 `vendor/gpui-base`、`vendor/gpui-component`。
- [ ] 提交一次未修改的原始内容，便于审阅后续补丁。
- [ ] 更新 `Cargo.toml` 的 `exclude`。
- [ ] 增加 `[workspace.dependencies]` 和 `[patch.crates-io]`。
- [ ] 新增 `vendor/PATCHES.md`，记录上游版本和补丁表。
- [ ] 保留 `LICENSE-APACHE`，修改过的文件按 Apache-2.0 要求标明。

退出：Cargo 能解析依赖图，只有一份 gpui。

### 任务 1.2：补丁分类（M）

已完成，结论在 `vendor/PATCHES.md`。规范化格式后，56 个差异文件中只有 24 个有真实改动。F 类默认不重放的判断对格式差异成立，但列表分区、Select 和 gutter 裁剪这 3 项是真实改动，已改为重放。按钮的自定义颜色 0.8.1 已自带。

- [ ] 用 `a9a7341c` 与 `vendor/gpui-component/crates/ui/src` 逐文件 diff。
- [ ] 按设计第 7 节 A 到 F 分类，写入 `PATCHES.md`，每项写明原因和删除条件。
- [ ] F 类默认不重放。

退出：所有差异文件和 4 个语言目录全部有归类。

### 任务 1.3：迁移底层 crate（L）

已完成。展示页 L2 对比基线：全窗口 307 万像素差异为 0，42 个组件区域和布局全部一致。比对之外，0.8.1 的 `sizing`、`radio`、`popover_style` 和 `Select` 箭头也有默认值变化，都已还原并记入 `PATCHES.md`。合入旧补丁时不要依赖 `patch` 的自动判断：它把 0.8.1 已有的按钮修复反向撤销过一次，只有逐行审 diff 才能发现。

- [ ] `vendor/gpui-markdown-editor`：切到 `gpui-pre`。
- [ ] `crates/yttt-terminal`：切到 `gpui-pre`，迁移滚动事件和文字 API。
- [ ] `crates/yttt-ui`：切换依赖，修复编译。
- [ ] 重放 C 类呈现补丁到 `vendor/gpui-component`。

退出：三个 crate 各自 `cargo check` 通过，测试通过。

### 任务 1.4：迁移代码编辑器（L）

已完成。语言支持全部移到 yttt 侧，不需要 vendor 补丁。vim 搜索丢字符的根因是上游搜索面板把自身回显当成新激活。

- [ ] 重放 A、B 类补丁到 `vendor/gpui-base`，每个 API 带测试。
- [ ] 修改 `src/ui/editor/view.rs`，改用 `EditorState`。
- [ ] 迁移 `document.rs`、`vim.rs`、`readonly_code_view.rs`、`layout_editor_controller.rs`、`render.rs`。
- [ ] 重放 D 类语言支持，确认 `LanguageRegistry` 能否替代。

退出：现有编辑器和 vim 测试全部通过。

### 任务 1.5：迁移主应用（L）

已完成。`desktop_tray_lifecycle` 的 2 个失败只出现在本机，`master` 上同样如此，CI 的 macOS 和 Linux 都通过，属于本机环境问题，根因未查。

- 右键菜单：0.8.1 在鼠标事件里打开菜单，排队的终端聚焦会抢走焦点。右键不再排队聚焦，workbench 也会等一帧再判断焦点是否丢失。
- 窗口不可见：新 GPUI 不出帧，桌面端要到窗口可见才 attach 终端。终端进程由 Host 启动，不受影响，所以接受这个行为。
- Breaking Changes：0.6 到 0.8.1 的说明涉及 Plot、DatePicker、WebView，yttt 都没有使用，其余由编译器覆盖。

- [ ] 迁移 `Root`、`TitleBar`、`ThemeConfig`、`set_locale`、`rebind_keybindings`。
- [ ] 迁移 `to_gpui_component_theme_config` 等主题映射，核对 highlight theme。
- [ ] 修复 `list`、`select`、`menu`、`notification`、`dialog` 的 API 变化。
- [ ] 逐版本核对 0.6 到 0.8.1 发布说明的 Breaking Changes。yttt 目前未调用 `is_eof`，也未使用 `Divider`，这两项不用迁移。
- [ ] 更新 `clippy.toml` 里变更的路径。

退出：L0 通过。

### 任务 1.6：抑制默认行为变化（S）

已完成，方式和下方清单不同：没有逐个调用 `.scroll_bounce(false)`，而是把 `ScrollBounce::enabled_by_default` 改为 false，没有开关的组件也一并覆盖。Tooltip、Dialog、Tabs 的动画在迁移前就已存在，不算新行为。

- [ ] 所有使用到的 `List`、`Tree`、`Select`、`Scrollable`、`Command` 设置 `.scroll_bounce(false)`。
- [ ] 找出没有开关的组件，记录到 P2、P3 的替换清单。
- [ ] 增加回归测试，确认这些列表不回弹。

### 任务 1.7：全平台验证（M）

CI 已通过（提交 `a3d10d9`，Run 41287845986）：macOS 917 过，Linux 1487 过，Windows 242 过，三平台构建成功。Windows 只跑 core contracts，不跑完整 UI 测试。L2 全窗口 0 差异，性能复测通过，见 `baselines/gpui-kit-spike.md`。剩余项：手工清单、Windows 标题栏。

- [ ] macOS、Windows、Linux CI 全部通过。
- [ ] 跑 L2：与 P0 截图比较，差异全部解释。
- [ ] 跑手工回归清单。
- [ ] 跑终端性能，与基线比较。
- [ ] 打包脚本验证：`scripts/build-macos-dmg.sh`、Linux tar、Windows installer。
- [ ] 更新 `CHANGELOG.md`。

**P1 退出门禁：**

- L0 和三平台 CI 通过。
- L2 无未解释的差异。
- 终端性能满足 1.0 的条件。
- IME、vim、Windows 标题栏清单全部通过。
- `PATCHES.md` 与实际 diff 一致。

**回滚：** 分支合入前不影响 `master`；合入后整体 revert 这个 PR。

## 4. P2：收拢 seam 与 token

目标：所有 UI 入口经过 `yttt-ui`。业务代码基本不再感知 `gpui_component`，视觉基本不变。

结果（2026-11-01）：已完成，和下方清单有四处偏差。

- 2.1：没有写 `YtttRootPlugin`。0.8.1 的根已经从主题同步 rem，手动同步全部删除，`yttt_root` 只是唯一入口。
- 2.2：只加了文字阶梯和两级 elevation。motion token 没有使用方，等 P3 的 tooltip。`metrics.toml` 用 `tests/ui_metrics.rs` 校验，未对齐项显式标记。
- 2.4：`YtttMenuItem` 保留 builder 写法，内部是纯数据。`YtttMenu::into_popup_menu` 给文件树用，P6 换掉 `tree` 后收回。
- 2.5：只收口 `init`、`Icon`、`IconName`、`Kbd`。**P2 退出门禁没有完全达到**：业务代码还有 71 处 `gpui_component`，主要是 `input`、`select`、`list`、`tab`、`highlighter`。不加全量重导出，因为那只是改名，这些类型在 P3 到 P8 被替换时自然消失。

### 任务 2.1：窗口入口（M）

- [ ] 新增 `crates/yttt-ui/src/window.rs` 和 `YtttRootPlugin`。
- [ ] 为 rem 同步、根背景、窗口边框写测试。
- [ ] 迁移至少 12 处 `Root` 构造：`app/mod.rs`、`existing_host.rs`、`remote_connect.rs`、`font_options.rs`、`onboarding.rs`、`command_center.rs`。

退出：`src/` 里没有 `Root::new`。

### 任务 2.2：token 扩展（M）

- [ ] 增加 elevation、text scale、motion token。
- [ ] 替换 primitives 中的字面量尺寸和颜色。
- [ ] 新增 `tests/visual/zed/metrics.toml`，L1 对比 `UiStyle::zed()`。
- [ ] 确认 `Rounded` 能正常工作。

退出：L1 能运行；未对齐的项目显式列出，不作失败处理，直到所在阶段完成。

### 任务 2.3：surface 模块（S）

- [ ] 新增 `surface.rs` 和 elevation 渲染。
- [ ] 替换 tooltip、dialog、palette 等处的手写 surface。

### 任务 2.4：menu interface（M）

- [ ] 新增 `menu.rs` 的 `MenuEntry` 模型。
- [ ] 实现暂时落在 `gpui_component::menu` 上。
- [ ] 把 8 个非测试文件的调用迁移到新 interface。

退出：业务代码不再出现 `gpui_component::menu`。

### 任务 2.5：其余收口（M）

- [ ] `Icon`、`IconName` 通过 `yttt_ui::icon`。
- [ ] `Kbd`、`Spinner`、`scroll`、`list`、`searchable_list`、`tooltip`、`notification` 通过 `yttt-ui`。
- [ ] `clippy.toml` 增加对应禁用规则。

**P2 退出门禁：** L0、L2 通过；除 `src/ui/theme`、`src/ui/editor` 的白名单外，业务文件不再引用 `gpui_component`；白名单写入 `clippy.toml` 注释。

## 5. P3：基础控件

进度（2026-11-01）：3.1 的按钮部分完成。按钮改到 `gpui_base::Button` 后 L2 全窗口 0 差异，L1 覆盖 4 种高度。文字大小已按 Zed 1.26.2 信任弹窗的实测对齐为 14px，内边距 4px 本来就一致；同一文案宽度 119 对 118。Zed 基线截图存于 `tests/visual/zed/baseline-1.26.2/`，由 `scripts/zed-input.swift` 驱动，需要终端的屏幕录制和辅助功能权限。命令面板实测约 607px，和 1.12 源码的 34rem 不同，以实测为准。

3.3 Tooltip 的几何已与 Zed 一致（高 30.5px、圆角约 4px、文字 14px），差距在配色。默认 One Dark 已改用 Zed 配色，同一截图链路下面板、选中、文字三色和 Zed 基线逐字节相同。语法高亮颜色留到 P7。3.5 的开关已按 `settings_general.png` 改成单层 32×20，同一截图链路下关闭态和滑块颜色与 Zed 一致；Radio 未改。3.4 只把 Search 边框改为 `border`。`screencapture` 带显示器色彩配置，基线 PNG 的颜色值不能当主题值用。`yttt_menu_icon_button` 留到 P4。展示页高度改为 900，L2 基线随视觉变化重新生成。

每个任务的步骤相同，以下称**标准流程**：

1. 采集 Zed 目标状态，确认 L1 指标。
2. 先写行为测试。
3. 用 `gpui-base` 替换 primitive 实现，函数名不变。
4. 对齐 token，更新 `metrics.toml`。
5. 跑 L1、L2、L3，提交报告。

| 任务 | 组件 | 大小 | 额外要求 |
|---|---|---|---|
| 3.1 | Button、IconButton、ButtonGroup | M | 增加 Compact、Medium、Large |
| 3.2 | Label、Kbd、Keybinding hint、Divider | S | 对齐文字阶梯 |
| 3.3 | Tooltip | S | 用 `TooltipDefaults` 安装 500ms 延迟（按基线实测） |
| 3.4 | Input、Number input、Search input | M | 中文 IME、placeholder、focus ring |
| 3.5 | Switch、Radio、Checkbox | S | 保留标签函数 |
| 3.6 | Tabs | M | 边框表达选中态，关闭按钮显隐 |
| 3.7 | Scrollbar | S | 与编辑器共享外观 |

**P3 退出门禁：** 所列组件的 L1、L3 通过或进入例外清单；Compact、Medium、Large 三种按钮均已覆盖。

## 6. P4：浮层与菜单

### 任务 4.1：Popover、Dialog、Alert（M）

- [ ] 替换为 `gpui-base` 的 Popover、Dialog、AlertDialog。
- [ ] 按基线 Zed 配置入场动画和遮罩。
- [ ] 验证焦点转移、Esc、遮罩点击。

### 任务 4.2：Notification（S）

- [ ] 基于 `ToastManager`、`ToastStackState` 重做，或保留现有实现但接入 surface。
- [ ] 判断 `always_show_close_button` 补丁能否删除。

### 任务 4.3：Context menu（L）

- [ ] 先写纯数据和交互测试：导航、禁用项跳过、子菜单、Esc、点击外部、窗口边缘避让、从父项移向子菜单时不误关闭。
- [ ] 用 `Popup`、`Positioner`、`FocusTrapElement` 实现。
- [ ] 替换 P2 的过渡实现，调用方不改。
- [ ] 删除 `PopupMenuStyle`、`MenuItemAppearance` 补丁。

### 任务 4.4：Dropdown menu 与标题栏菜单（S）

- [ ] 复用 4.3。
- [ ] 确认原生菜单不受影响。

**P4 退出门禁：** L1、L3 通过；`PATCHES.md` 中 C 类菜单补丁已删除。

## 7. P5：Picker 与 Select

### 任务 5.1：下沉 Picker（M）

- [ ] 把 `src/ui/palette/picker` 移到 `crates/yttt-ui/src/picker`。
- [ ] 将 `palette_item` 的 row 外观合并进去。
- [ ] 去掉 picker 对业务类型的依赖。

### 任务 5.2：Command palette 对齐（M）

- [ ] 用 rem 表示 34rem 宽度。
- [ ] 对齐行高、匹配高亮、keybinding 显示、footer。
- [ ] 用虚拟列表验证大列表性能。

### 任务 5.3：Select 与带搜索选择（M）

- [ ] 基于 `gpui_base::Select`、`gpui_base::Combobox`。
- [ ] 复用 picker row。
- [ ] 替换字体选择等现有调用。

### 任务 5.4：SSH 项目选择器（M）

- [ ] 评估能否接入 `PickerDelegate`。
- [ ] 能接入就迁移；不能接入就保留专用 view，只共享 row 和 surface，并记录原因。

**P5 退出门禁：** palette、select 的 L1、L3 通过；`hover_matches_selected` 补丁删除。

## 8. P6：文件树

### 任务 6.1：行外观（M）

- [ ] 新增 `crates/yttt-ui/src/tree.rs` 的 `yttt_tree_row`。
- [ ] 对齐行高、缩进、图标、Git 颜色、hover、选中和重命名输入。

### 任务 6.2：切换到 `gpui_base::TreeState`（M）

- [ ] 用 `TreeState` 替换 `gpui_component::tree`，业务状态保留在 `ProjectFileTree`。
- [ ] 保证刷新后展开和选中状态不丢。
- [ ] 验证懒加载、隐藏文件、文件操作的现有测试。
- [ ] 接入 P4 的 context menu。

### 任务 6.3：Indent guides（S）

- [ ] 新增绘制逻辑，只绘制可见范围。
- [ ] 随激活路径高亮，行为以基线 Zed 为准。

### 任务 6.4：Sticky 目录头（M，可延后）

- [ ] 滚动时固定祖先目录，点击跳回。
- [ ] 不影响虚拟列表性能。

**P6 退出门禁：** L1、L3 通过；1 万文件目录下的滚动无明显退化。

## 9. P7：编辑器与 Markdown

### 任务 7.1：编辑器外观（M）

- [ ] 接入 `set_scrollbar_renderer`。
- [ ] 对齐 gutter、当前行、缩进线、搜索条。
- [ ] 打包 Lilex，更新 `assets` 和字体列表，保留 OFL 文件。
- [ ] 重新评估 `flush_search_panel` 能否由 yttt 自行组合。

### 任务 7.2：Markdown 编辑主题（M）

- [ ] 补全 `to_markdown_editor_theme`。
- [ ] 对齐标题、段落、列表、代码块、表格、引用块。
- [ ] 主题表达不了时才改 `vendor/gpui-markdown-editor`，并记录到 `PATCHES.md`。

### 任务 7.3：Markdown 只读预览（S）

- [ ] 检查 `src/ui/editor/preview.rs` 当前实现。
- [ ] 判断是否改用 `gpui_base::text`，结论写进 PR。

**P7 退出门禁：** L3 通过或进入例外清单；IME 和 vim 手工清单通过。

## 10. P8：Shell 与业务页面

| 任务 | 内容 | 大小 |
|---|---|---|
| 8.1 | 标题栏：高度、padding、项目与分支样式 | S |
| 8.2 | 状态栏：按钮、分组、分隔线 | S |
| 8.3 | 面板、分割线、Dock 激活态 | S |
| 8.4 | Settings 页只替换 primitive，不重排布局 | M |
| 8.5 | Agent 侧边栏、Command Center 应用 token，不进入 L3 | M |
| 8.6 | 终端字体：验证 Lilex 加 Nerd Font 回退，结论写进 PR；默认值不改 | S |
| 8.7 | Windows 标题栏回归，按 `docs/windows-titlebar-verification.md` | S |

**P8 退出门禁：** L3 主窗口对比通过或进入例外清单；Windows 标题栏回归通过。

## 11. P9：全量验收与收尾

### 任务 9.1：依赖收紧（M）

- [ ] 根 crate 移除 `gpui-component` 依赖，业务代码靠编译器保证不再使用。
- [ ] 统计 `yttt-ui` 对 `gpui-component` 的剩余使用，决定是否移除，结论写入设计文档。
- [ ] 删除 `vendor/gpui-component` 中已无用的补丁；如果清零，移除 `[patch.crates-io]` 对应条目。

### 任务 9.2：全量验收（M）

- [ ] 全部组件跑 L1、L2、L3。
- [ ] 审核 `exceptions.toml`，每条都有原因和批准人。
- [ ] 三平台手工清单。
- [ ] 终端性能与 P0 基线对比。
- [ ] 按 `docs/superpowers/` 现有习惯记录结果。

### 任务 9.3：文档与发布（S）

- [ ] 更新 `README.md`、`README.zh-CN.md` 截图。
- [ ] 更新 `CHANGELOG.md`。
- [ ] 准备发布由用户确认后进行。

**P9 退出门禁：**

- `src/` 不再引用 `gpui_component`。
- 所有组件的 L1 完全一致，L3 在阈值内或有批准的例外。
- `PATCHES.md` 只留下有明确理由的补丁。
- 用户在固定基线配置下对比确认。

## 12. 上游协作（需批准后才执行）

可在 P1 之后的任意时间进行，不阻塞主线：

1. 整理 A 类编辑器 API 的通用性说明。
2. 拆成小 PR，每个带测试。
3. 合入后升级 `gpui-base`，删除对应补丁。

## 13. 风险检查点

| 时点 | 检查 | 不满足时 |
|---|---|---|
| 任务 1.0 结束 | 性能、IME、vim | 停止，与用户讨论退回方案 |
| P1 合入前 | 全平台、L2、手工清单 | 不合入 |
| P3 完成后 | L3 校准是否合理，例外数量是否失控 | 调整阈值或范围，先和用户确认 |
| P6 完成后 | 用户在实际使用中对比一次 | 调整 P7、P8 优先级 |
| 上游发布新版本 | 是否存在必须采纳的修复 | 独立任务，不打断进行中的阶段 |

## 14. 待用户决定

1. 根目录 LICENSE：在下一次发布前决定。
2. 是否向 gpui-kit 上游提交编辑器 API 补丁。

这两项都不阻塞 P0 和 P1。
