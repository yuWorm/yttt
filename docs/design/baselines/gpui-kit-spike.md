# gpui-kit 0.8.1 可行性验证（任务 1.0）

- 分支：`spike/gpui-kit-0.8.1`，基于 `feat/gpui-kit-p0-baselines`
- 日期：2026-11-01
- 状态：继续条件全部满足

## 已验证

### 工具链

`gpui-pre 0.3.10` 不能用 Rust 1.96 或 1.97 编译（`result_option_map_or_default` 未稳定），1.98.0 可以。这是设计文档没有列出的前提。分支已把 `rust-toolchain.toml` 改为 1.98.0；全 workspace 在新工具链上的 clippy 尚未运行。

### `yttt-terminal` 迁移

`cargo test -p yttt-terminal` 通过，改动很小：

| 改动 | 处数 |
|---|---|
| `paint_image` 增加 `image_bounds` 参数 | 2 |
| `Modifiers::secondary` 和 `secondary_key` 需要 `cx` | 4 |
| `KeyBinding::new` 需要 `&'static str` | 1 |
| `ScrollWheelEvent` 改名，并增加 `source` 字段 | 3 处调用与 2 处测试 |

`repaint_batches_contiguous_cjk_into_one_double_width_paint_call` 在 6 次全量运行里失败 1 次，失败输出没有保留，后续 5 次和单独运行都通过。基线分支 3 次运行都通过，样本不足以判断是否由升级引入。任务 1.3 要用更多次运行定位。

### 终端性能（直连后端）

相同命令和机器，每组 3 次。基线见 `terminal-perf-a2160c8.md`。

| 场景 | 指标 | 基线 | 0.3.10 | 变化 |
|---|---|---:|---:|---:|
| burst | MiB/s 中位 | 85.43 | 95.18 | +11% |
| burst | paint p95 中位 ms | 2017.46 | 2009.07 | -0.4% |
| damage | paint p95 中位 ms | 17.60 | 18.37 | +4.4% |
| full | paint p95 中位 ms | 17.55 | 18.51 | +5.5% |
| scroll | paint p95 中位 ms | 17.60 | 18.50 | +5.1% |
| interactive | paint p95 中位 ms | 17.32 | 18.48 | +6.7% |
| interactive | input→PTY p95 ms | 0.058 | 0.059 | +1.7% |
| interactive | input→paint p95 ms | 16.089 | 13.722 | -15% |
| interactive | echo→paint p95 ms | 14.885 | 12.820 | -14% |

全部在 10% 以内。`damage` 的 33 ms 偶发帧仍然存在，升级前后都是 1/3 的运行命中。

**未覆盖：** Host 后端，见下文。

### 主应用迁移

全 workspace 已切到 `gpui-pre 0.3.10` 和 gpui-kit 0.8.1，只有一份 gpui，根 crate 编译通过。切换时根 crate 只有 108 个编译错误，其中约 80 个是编辑器本地 API。

补丁现状：

| 补丁 | 位置 | 状态 |
|---|---|---|
| 编辑器 API、光标形状、输入开关、撤销分组、显示行移动 | `vendor/gpui-base/src/input/base/yttt_ext.rs`，上游文件改动很少 | 已重放 |
| `replay_history` 返回改动起点 | `vendor/gpui-base/src/input/base/state.rs` | 已重放 |
| `TreeState::focus_handle` | `vendor/gpui-base/src/tree.rs` | 已重放 |
| `TooltipAppearance`、`always_show_close_button`、`rebind_keybindings`、搜索面板选择器 | `vendor/gpui-component` | 已重放 |
| `PopupMenuStyle` | | **未重放**，菜单现在是上游外观 |
| `hover_matches_selected`、`flush_search_panel`、主题字段 `accordion_hover`、`placeholder_foreground`、`tiles` | | **已删除调用**，有视觉差异，待任务 1.5 处理 |
| D 类语言支持 | | **未重放** |

`vendor/PATCHES.md` 还没写，任务 1.2 要补。

GPUI 新增的两点约束：`Keystroke::parse` 和 `KeyBinding::new` 要求 `&'static str`。yttt 改用 `parse_with_os`，并增加 `intern_keys` 给运行时快捷键做去重驻留。

### 测试

`cargo test -p yttt --tests`：865 通过，18 失败，1 忽略。

| 失败 | 数量 | 判断 |
|---|---|---|
| `desktop_tray_lifecycle` | 2 | master 上已失败 |
| 语言高亮与语言目录 | 8 | D 类未重放，预期内 |
| 通知关闭与动作 | 4 | 待查，`always_show_close_button` 已重放但测试仍失败 |
| `macos_runtime::application_platform_exposes_system_fonts` | 1 | 报 “Mac platform not created on main thread”，是 GPUI 新增的主线程断言，测试需调整 |
| `project_sidebar_context_menu_can_create_project` | 1 | 待查 |
| `macos_bundle` 重开 | 1 | 锁屏导致，解锁后通过 |
| `vim_search_reuses_the_query...` | 1 | 真实差异，见下文 |

vim 的撤销分组、撤销光标、可视模式、行操作和粘贴测试已全部通过，`editor_document` 为 24 过 1 失败。

## 已知问题

**vim 搜索首字符丢失。** 按 `/` 后马上输入 `one`，查询变成 `ne`。旧实现在动作内同步聚焦，0.8.1 的面板在 overlay 同步时才聚焦。测试里多等两帧也没有解决，所以还不能排除这在真实使用中可见。任务 1.4 必须修复。

**Host 路径未验证。** 2026-11-01 03:00 左右测试时屏幕处于锁定状态（`CGSSessionScreenIsLocked = 1`）。这时桌面端完成工作区打开后不再重绘，终端面板不创建，`macos_bundle` 和 Host 性能冒烟都超时；此前通过的直连冒烟在锁屏下也失败。基线的 `macos_bundle` 在同一时段通过，所以 0.3.10 下窗口不可见时不出帧是行为变化。

这带来一个独立的设计问题：yttt 在 `render` 里创建终端面板，新 GPUI 下窗口不可见就不会创建。普通使用影响不大，但窗口在后台启动、恢复多窗口时可能延迟。任务 1.5 要确认。

## 未验证

- `EditorState` 的真实窗口体验：编辑、高亮、滚动只跑过测试。
- 新工具链上的全 workspace clippy、Windows 和 Linux 编译。
- L2 视觉对比。菜单等已知有差异。

## Host 性能

早先的基线不能直接比较：同一天下午基线本身的 paint p95 从 17.7 ms 变成约 25.7 ms，机器上当时有远程控制和 OrbStack 负载。以下都是同一时段交替运行的结果。

| 场景 | 指标 | 基线 | 0.3.10 | 判断 |
|---|---|---:|---:|---|
| full | paint p95 ms（4 次） | 25.71、25.80 | 26.20、25.40 | 无差异 |
| interactive | paint p95 ms（各 2 次） | 25.35、25.17 | 25.26、25.41 | 无差异 |
| interactive | input→paint ms（各 2 次） | 27.43、28.20 | 26.07、27.31 | 略好 |
| scroll | paint p95 ms（各 2 次） | 27.57、28.08 | 28.33、25.36 | 无差异 |
| damage | paint p95 ms（各 2 次） | 28.67、28.84 | 25.13、26.77 | 略好 |
| burst | MiB/s 中位（各 7 次） | 131.86 | 111.77 | **-15%，超过 10%** |

Host、客户端、协议和传输 crate 没有代码改动，关键依赖版本基本一致。拆分 `burst` 的结果：

| 对比 | 中位 MiB/s | 差异 |
|---|---|---:|
| 基线 Rust 1.96 对 基线 Rust 1.98，交替 4 对 | 129.5 对 120.5 | 约 -7% |
| 基线 Rust 1.98 对 0.3.10，交替 5 对 | 116.55 对 109.67 | 约 -6% |

两组样本都有重叠，单次波动达 ±15%，所以这是方向，不是精确拆分。可以确定的是下降由工具链和 GPUI 两部分组成，没有单一修复点。Rust 1.98 是 `gpui-pre 0.3.10` 的硬前提。

`burst` 的 paint p95 两边都约 2000 ms，属于既有行为。未做 profile，GPUI 部分的具体原因不明。

## M1 复测

M1 提交 `e4777bc`，基线是用 Rust 1.98 编译的 P0。2026-11-01 同一时段交替运行，普通场景每边 3 次，`burst` 每边 6 次。

| 场景 | 指标 | 基线 | M1 | 变化 |
|---|---|---:|---:|---:|
| full | paint p95 ms | 32.87 | 32.37 | -1.5% |
| damage | paint p95 ms | 32.96 | 33.28 | +1.0% |
| scroll | paint p95 ms | 32.57 | 30.64 | -5.9% |
| interactive | paint p95 ms | 30.79 | 31.28 | +1.6% |
| interactive | input→PTY ms | 0.131 | 0.131 | 0 |
| interactive | input→paint ms | 32.62 | 33.10 | +1.5% |
| interactive | echo→paint ms | 25.92 | 25.97 | +0.2% |
| burst | MiB/s | 116.53 | 110.55 | -5.1% |

全部在 10% 以内。帧时间比早先高，是当时的机器负载，两边一致。`burst` 的单次范围是基线 106 到 124、M1 102 到 115，有重叠。

`burst` 只采到 M1 一侧的 `sample` 数据，基线采样失败，没有做差异对比。M1 的桌面和 Host 进程都以等待为主，没有明显 CPU 热点，所以不再继续 profile。

## 继续条件现状

| 条件 | 现状 |
|---|---|
| 终端性能在基线 10% 以内 | 通过。相对 Rust 1.96 基线的 `burst` -15% 已由用户接受，相对 Rust 1.98 基线为 -5.1%。解锁后 `macos_bundle` 通过，确认此前失败来自锁屏 |
| 中文 IME 可用 | 通过，用户在 `yttt-terminal` 上确认 |
| vim 依赖的 API 可在 `gpui-base` 中实现 | 通过，API 全部重放，只剩搜索首字符问题 |
