# 本地补丁

`vendor/gpui-base` 和 `vendor/gpui-component` 来自 crates.io 的 0.8.1 发布包，经 `Cargo.toml` 的 `[patch.crates-io]` 替换上游。原始内容见提交 `chore: vendor unmodified gpui-base and gpui-component 0.8.1`，用它和当前目录做 diff 就能得到全部本地改动。

- 上游：<https://github.com/longbridge/gpui-kit>，许可证 Apache-2.0，见各目录的 `LICENSE-APACHE`
- 改动过的文件在第一行标注 `Modified for yttt`
- 升级上游时，从新的发布包重新解开，再按本表重放

分类含义见 `docs/design/2026-10-31-gpui-kit-zed-visual-design.md` 第 7 节。

## gpui-base

| 补丁 | 文件 | 分类 | 原因 | 删除条件 |
|---|---|---|---|---|
| 编辑器 gutter 裁剪滚动内容 | `src/input/base/element.rs` | B | yttt 的 gutter 背景透明，上游靠不透明 gutter 盖住文字，文字会透出 | 上游提供裁剪 |
| 编程式编辑接口：`selection_anchor`、`selection_head`、`set_cursor_offset`、`set_anchored_selection`、`replace_range`、`previous_grapheme_boundary`、`next_grapheme_boundary`、`undo_edit`、`redo_edit`、`move_display_lines`、`move_to_search_match`、`search_panel_is_open` | `src/input/base/yttt_ext.rs`、`src/input/mod.rs`、`src/input/base/movement.rs` | A | vim 层用这些接口驱动编辑器 | 上游提供等价公开接口 |
| 撤销分组：`begin_history_group`、`end_history_group` | `src/input/base/yttt_ext.rs` | A | vim 复合命令要合成一次撤销 | 上游公开事务接口 |
| `replay_history` 返回改动起点 | `src/input/base/state.rs` | A | vim 撤销后光标要落在改动处，而不是恢复旧选区 | 上游提供撤销位置 |
| 文本输入开关：`set_text_input_enabled`、`text_input_enabled` | `src/input/base/yttt_ext.rs`、`src/input/base/state.rs` | A | vim 普通模式要拦截输入法和编辑动作 | 上游提供输入开关 |
| 光标形状：`InputCursorShape` | `src/input/base/yttt_ext.rs`、`src/input/base/state.rs`、`src/input/base/element.rs` | B | vim 普通模式显示块状光标 | 上游支持光标形状 |
| `TreeState::focus_handle` | `src/tree.rs` | B | 判断焦点是否在文件树内 | P6 改用 `gpui_base::TreeState` 的公开焦点接口 |

## gpui-component

| 补丁 | 文件 | 分类 | 原因 | 删除条件 |
|---|---|---|---|---|
| 重新导出 `InputCursorShape` | `src/input/mod.rs` | A | 业务代码经 component 使用 | 随 gpui-base 的光标补丁删除 |
| `rebind_keybindings` | `src/lib.rs` | B | `cx.clear_key_bindings()` 之后恢复组件快捷键，用户改键时需要 | 上游提供重绑接口 |
| 搜索面板 debug selector | `src/input/search.rs` | E | 测试定位面板 | 测试不再依赖该 selector |
| 搜索面板紧凑样式 | `src/input/search.rs` | C | 替代旧的 `flush_search_panel`。0.8.1 面板已贴边，只需还原样式 | P7 对齐编辑器时重做 |
| 搜索框回显判断去掉 anchor 条件 | `src/input/overlay.rs` | E | 上游 bug：输入首字符后面板重新全选，下一键覆盖它 | 上游修复。适合提 PR |
| `TooltipAppearance` | `src/tooltip.rs` | C | Zed 风格的快捷键提示颜色 | P3 用 `gpui-base` 重做 tooltip |
| `Notification::always_show_close_button` | `src/notification.rs` | C | 错误通知要始终显示关闭按钮 | P4 重做通知 |
| `PopupMenuStyle`、`MenuItemAppearance`，带样式时恢复 1px 边框 | `src/menu/*` | C | 0.8.1 的 `popover_style` 去掉了边框，改用阴影环 | P4 自有 context menu |
| `ListItem::hover_matches_selected`，选中背景 | `src/list/list_item.rs` | C | 文件树和列表的选中外观 | P5、P6 自有行外观 |
| 各分区标题独立测量 | `src/list/list.rs`、`src/list/cache.rs`、`src/list/delegate.rs` | C | SSH 列表有多个分区 | 上游支持，或 P5 不再使用 `List` |
| `Select` 触发器 hover，箭头固定 XSmall | `src/select.rs` | C | 保持现有下拉外观 | P5 自有 select |
| 中号和大号输入框内边距 12、16 | `src/sizing.rs` | C | 0.8.1 改成 10、12，文字左移 2px | P3 用 token 定义 |
| Radio 行高 1，去掉指示器上边距 | `src/radio.rs` | C | 0.8.1 的 Radio 变高 | P3 自有 radio |
| 主题 `placeholder_foreground` | `src/theme/schema.rs`、`src/theme/theme_color.rs`、`src/input/input.rs` | C | 占位符颜色独立于 muted 文本 | P3 自有 input |

## 待重放

旧 vendor 里的下列补丁在 0.8.1 没有等价实现，M1 要保留。

| 补丁 | 旧文件 | 分类 | 现状 | 任务 |
|---|---|---|---|---|
| 主题字段 `accordion_hover`、`tiles` | 无 | C | 上游改名或移除，赋值已删除，要找对应字段 | 1.5 |

## yttt 侧替代

dockerfile、hcl、nix、scss、vue 和 JSX 高亮由 `src/ui/editor/language.rs` 通过 `LanguageRegistry` 注册，查询文件在 `src/ui/editor/queries/`，不再需要 vendor 补丁。

## 不重放

基线是上游 `a9a7341c`。旧 `crates/ui/src` 与它共有 56 个文件存在差异，外加 4 个语言目录。两边用 rustfmt 规范化后，只有 24 个文件有真实改动。

| 文件 | 原因 |
|---|---|
| `input/clear_button.rs` | 0.8.1 已改用 `IconName::Close` |
| `button/button.rs` 的自定义 hover、active、前景色 | 0.8.1 已采用同样的实现 |
| `lib.rs` 的 `pub mod actions` | `gpui_base::actions` 已公开，yttt 没有直接使用 |
| `title_bar.rs` | 只有一行注释。Windows 标题栏行为在任务 1.7 验证 |
| `history.rs` 的 `start_grouping_new_version` | 改由 `undo_manager` 事务承接 |
| `theme/schema.rs` 的 Aurora 测试夹具 | 只改了测试 |
| 其余 32 个文件 | 仅格式差异：`dock/tiles.rs`、`dock/state.rs`、`window_border.rs`、`actions.rs`、`element_ext.rs`、`dialog/*`、`resizable/resize_handle.rs`、`popover.rs`、`breadcrumb.rs`、`clipboard.rs`、`link.rs`、`collapsible.rs`、`searchable_list/adapter.rs`、`button/dropdown_button.rs`、`list/separator_item.rs`、`input/display_map/*`、`input/lsp/*`、`input/popovers/diagnostic_popover.rs`、`input/blink_cursor.rs`、`highlighter/diagnostics.rs`、`setting/fields/*`、`plot/*`、`time/utils.rs`、`text/text_view.rs` |

`crates/assets`、`crates/macros` 没有需要保留的 yttt 改动，由 `gpui-kit-assets` 和 0.8.1 的宏 crate 取代。

## 许可证

`LICENSE-APACHE` 保持原样。yttt 自己新增的 `yttt_ext.rs` 同样按 Apache-2.0 分发。
