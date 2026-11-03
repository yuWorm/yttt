# gpui-kit 迁移与 Zed 视觉对齐实施计划

- 日期：2026-10-31
- 设计文档：[2026-10-31-gpui-kit-zed-visual-design.md](2026-10-31-gpui-kit-zed-visual-design.md)
- 状态：M1、P2 已合入，P3 和 P3.5 完成，待开 PR；P0 剩余项见下方进度

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

3.3 Tooltip 的几何已与 Zed 一致（高 30.5px、圆角约 4px、文字 14px），差距在配色。默认 One Dark 已改用 Zed 配色，同一截图链路下面板、选中、文字三色和 Zed 基线逐字节相同。语法高亮颜色留到 P7。3.5 的开关已按 `settings_general.png` 改成单层 32×20，同一截图链路下关闭态和滑块颜色与 Zed 一致；Radio 未改。3.4 只把 Search 边框改为 `border`。3.6 的标签结构本来就和 Zed 一致，只改了边框色。3.7 只对齐滑块颜色；Zed 编辑器滑块约 14px 宽、无圆角，几何留到 P7 的 `set_scrollbar_renderer`。3.2 的快捷键走 `yttt_kbd`，Zed 风格为无底框的 muted 文字。3.1 的 hover 只有一个样本：Zed `#2b3039`、yttt 约 `#2f353d`，在阈值内，不调整。Radio 没有 Zed 对应控件，只继承配色。`screencapture` 带显示器色彩配置，基线 PNG 的颜色值不能当主题值用。`yttt_menu_icon_button` 留到 P4。展示页高度改为 900，L2 基线随视觉变化重新生成。

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

## 5.5 P3.5：控件换到 `gpui-base`

P3 只对齐外观时，开关和标签仍是自绘 `div`，缺少键盘和无障碍语义，所以补做这一阶段。结果（2026-11-02）：

- 开关：改用 `gpui_base::Switch`，支持键盘、焦点样式和 Switch 角色，有行为测试。
- 标签：改用 `gpui_base::Tab`，获得 Tab 角色。`gpui_base` 的注释说明它还没有键盘导航，所以这一项没有键盘收益。项目标签文字改为 14px。
- Radio：`gpui_component::Radio` 内部已经是 `gpui_base::Radio`，`RadioGroup` 也依赖它，不重写，只删本地补丁。
- Tooltip：`TooltipAppearance` 补丁多余，已删除。延迟沿用上游 500ms，未接入新的 token。
- Input：底层已是 `gpui-base`，不动。Select 在 P5，菜单在 P4。

gpui-shell 是 JavaScript 插件运行时，不是主题系统，不在本计划范围内。主题 token 在 `gpui-base`，不依赖 shell。

## 6. P4：浮层与菜单

进度（2026-11-02）：右键和下拉菜单改由 `yttt_ui::menu::MenuLayer` 渲染，基于 `gpui_base::Positioner`，每个窗口一个。文件树仍用 Tree 自带的 `PopupMenu`，P6 替换后才能删除 `PopupMenuStyle` 补丁。子菜单未实现，当前菜单都不需要。

L3（2026-11-02）：用 `context_menu_hover.png` 离线比对菜单，行距 22.5、文字缩进 10、分组间距约 35 一致。菜单项右侧的快捷键已补上，从实际绑定读取，文件树和新菜单层都显示。菜单宽度随内容变化，未比较。Dialog 和通知没有 Zed 基线，未比对。

`gpui_component` 的 Dialog 和通知本来就建在 `gpui_base::Dialog` 与 `ToastManager` 上，不重写。`yttt_dialog_overlay` 的 9 处自绘遮罩加了 `gpui_base` 焦点陷阱。通知补丁保留，原因见 `vendor/PATCHES.md`。


### 任务 4.1：Popover、Dialog、Alert（M）

- [x] Dialog、AlertDialog 已基于 `gpui-base`，自绘遮罩加焦点陷阱。yttt 没有使用 Popover。
- [ ] 按基线 Zed 配置入场动画和遮罩。
- [x] 验证焦点转移、遮罩点击。

### 任务 4.2：Notification（S）

- [x] 保留现有实现。
- [x] `always_show_close_button` 补丁保留。

### 任务 4.3：Context menu（L）

- [x] 交互测试：导航、禁用项跳过、Esc、点击外部、窗口边缘避让。子菜单未实现，当前菜单不需要。
- [x] 用 `Positioner` 实现。
- [x] 替换 P2 的过渡实现，调用方不改。
- [ ] 删除 `PopupMenuStyle`、`MenuItemAppearance` 补丁（移到 P6）。

### 任务 4.4：Dropdown menu 与标题栏菜单（S）

- [x] 复用 4.3。
- [x] 确认原生菜单不受影响：yttt 没有调用 `set_menus`。

**P4 退出门禁：** L1、L3 通过。C 类菜单补丁的删除移到 P6，因为文件树要到换掉 Tree 才脱离 `PopupMenu`（2026-11-02 确认）。

## 7. P5：Picker 与 Select

### 任务 5.1：下沉 Picker（M）

- [x] 把 `src/ui/palette/picker` 移到 `crates/yttt-ui/src/picker`。
- [x] 将 `palette_item` 的 row 外观合并进去。
- [x] 去掉 picker 对业务类型的依赖。`PalettePickerDelegate` 和按 `PaletteKind` 决定的图标、状态留在应用侧。

### 任务 5.2：Command palette 对齐（M）

- [x] 宽度按实测 607px，随 UI 字号缩放（37.9375rem）。
- [x] 对齐行高、keybinding 显示、footer。`yttt-ui-gallery --scene palette` 按 `palette.png` 的布局渲染面板，面板宽高、行距、底栏一致，头部 37.5 对 Zed 约 36。
- [x] 匹配高亮：标题里第一处命中用强调色。Zed 是模糊匹配，可以标出不连续的字符；yttt 的过滤是子串匹配，所以只标一段。
- 修复：`Input::h` 只对多行输入生效，单行输入框的配置高度此前从未生效，一律是 32。现在走 `Styled::h`。只有面板输入框有 Zed 实测（28），其余配置改为实际在用的 32，外观不变。
- [x] 大列表性能：原先每行约 11 µs，2000 行每帧 22.8 ms。现在 64 行以上、行高一致的列表只渲染可见行，2000 行 0.54 ms，2 万行 1.7 ms（release 构建、测试平台）。行高不一致的列表仍全部渲染。

### 任务 5.3：Select 与带搜索选择（M）

- [x] `gpui_component::Select` 已基于 `gpui_base::Select`，不重写。改用 Zed 的上下双箭头，Zed 风格下宽度随内容，删除箭头补丁，hover 补丁保留。
- [x] 不复用 picker row：下拉列表由 `gpui_component::Select` 内部渲染，复用需要重写弹出层。
- [x] 字体选择等调用都已走 `yttt_select`。

### 任务 5.4：SSH 项目选择器（M）

- [x] 评估：不能接入。它依赖 `List` 的分区、搜索和键盘导航，`ListDelegate` 要求返回 `ListItem`，`PickerDelegate` 只有扁平列表。
- [x] 保留专用 view。行尺寸已共用 palette 的样式参数，面板共用 `yttt_panel`。

**P5 退出门禁：** palette、select 的 L1、L3 通过。`hover_matches_selected` 只有文件树使用，删除移到 P6。

结果（2026-11-03）：L1 指标全部对齐。L3 用 `--scene palette` 对照 `palette.png`，用展示页对照 `settings_general.png` 的下拉框。真实窗口未手工验证。

## 8. P6：文件树

### 任务 6.1：行外观（M）

- [x] 新增 `crates/yttt-ui/src/tree.rs` 的 `yttt_tree_row`。
- [x] 对齐行高 26、缩进 20、图标 14、hover、选中和重命名输入。Zed 风格不显示展开箭头，获得焦点的选中行只画描边。Git 颜色保持原样：基线里的 fixture 在被忽略的目录，Zed 画成灰色，和 yttt 对忽略文件的处理一致。

### 任务 6.2：切换到 `gpui_base::TreeState`（M）

- [x] `TreeState` 已来自 `gpui_base`。文件树改为直接使用 `gpui_base::Tree`，不再经过 `gpui_component::tree`。
- [ ] 保证刷新后展开和选中状态不丢。
- [ ] 验证懒加载、隐藏文件、文件操作的现有测试。
- [x] 接入 P4 的 context menu，删除 `PopupMenuStyle`、`MenuItemAppearance`、`hover_matches_selected` 和 `DropdownMenu` 补丁。

### 任务 6.3：Indent guides（S）

- [x] 每行绘制祖先层级的引导线，随虚拟列表只绘制可见行。
- [ ] 随激活路径高亮：未做，基线没有展示这一状态。

### 任务 6.4：Sticky 目录头（M，可延后）

- [ ] 滚动时固定祖先目录，点击跳回。
- [ ] 不影响虚拟列表性能。

**P6 退出门禁：** L1、L3 通过；1 万文件目录下的滚动无明显退化。

结果（2026-11-03）：L1 新增行高、缩进、图标三项并对齐。L3 用 `yttt-ui-gallery --scene tree` 对照 `main2.png`。1 万项、10 层嵌套的树每帧约 0.21 ms（release、测试平台），只渲染可见行。

## 9. P7：编辑器与 Markdown

### 任务 7.1：编辑器外观（M）

- [ ] 接入 `set_scrollbar_renderer`。
- [x] 对齐 gutter 和当前行：行号右缘 58、正文起点 92（按字号缩放，vendor 补丁 `set_gutter_padding`），当前行与行号颜色取自 Zed One Dark。缩进线、搜索条未做。
- [x] 打包 Lilex 2.700 四个字重和 OFL。Zed 风格编辑器默认用 Lilex。
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
