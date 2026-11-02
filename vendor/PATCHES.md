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
| `ScrollBounce::enabled_by_default` 恒为 false | `src/scroll_bounce.rs` | C | 0.8 起 macOS 默认回弹，Zed 不回弹。各组件默认值都经过这里，`Sidebar`、设置页、`TextView` 没有单独开关 | 有全局配置，或 yttt 决定采用回弹 |
| `Button` disabled 时清除 hover、active 样式 | `src/button.rs` | B | disabled 按钮不应有指针反馈 | 上游提供，或 `ButtonStyles` 支持 |
| `TreeState::focus_handle` | `src/tree.rs` | B | 判断焦点是否在文件树内 | P6 改用 `gpui_base::TreeState` 的公开焦点接口 |

## gpui-component

| 补丁 | 文件 | 分类 | 原因 | 删除条件 |
|---|---|---|---|---|
| 重新导出 `InputCursorShape` | `src/input/mod.rs` | A | 业务代码经 component 使用 | 随 gpui-base 的光标补丁删除 |
| `rebind_keybindings` | `src/lib.rs` | B | `cx.clear_key_bindings()` 之后恢复组件快捷键，用户改键时需要 | 上游提供重绑接口 |
| 搜索面板 debug selector | `src/input/search.rs` | E | 测试定位面板 | 测试不再依赖该 selector |
| 搜索面板紧凑样式 | `src/input/search.rs` | C | 替代旧的 `flush_search_panel`。0.8.1 面板已贴边，只需还原样式 | P7 对齐编辑器时重做 |
| 搜索框回显判断去掉 anchor 条件 | `src/input/overlay.rs` | E | 上游 bug：输入首字符后面板重新全选，下一键覆盖它 | 上游修复。适合提 PR |
| `DropdownMenu` 接受 `gpui_base::Button` | `src/menu/dropdown_menu.rs` | C | yttt 按钮已改用 `gpui_base::Button` | P4 自有菜单 |
| `Notification::always_show_close_button`，图标和关闭按钮内联布局，正文 muted 色，关闭按钮 selector | `src/notification.rs` | C | 错误通知要始终显示关闭按钮，长文本换行不与按钮重叠 | P4 重做通知 |
| `PopupMenuStyle`、`MenuItemAppearance`，带样式时恢复 1px 边框 | `src/menu/*` | C | 0.8.1 的 `popover_style` 去掉了边框，改用阴影环 | P4 自有 context menu |
| `ListItem::hover_matches_selected`，选中背景 | `src/list/list_item.rs` | C | 文件树和列表的选中外观 | P5、P6 自有行外观 |
| 各分区标题独立测量 | `src/list/list.rs`、`src/list/cache.rs`、`src/list/delegate.rs` | C | SSH 列表有多个分区 | 上游支持，或 P5 不再使用 `List` |
| `Select` 触发器 hover，自身箭头用 XSmall | `src/select.rs` | C | 保持现有下拉外观 | P5 自有 select |
| 中号和大号输入框内边距 12、16 | `src/sizing.rs` | C | 0.8.1 改成 10、12，文字左移 2px | P3 用 token 定义 |
| 主题 `placeholder_foreground` | `src/theme/schema.rs`、`src/theme/theme_color.rs`、`src/input/input.rs` | C | 占位符颜色独立于 muted 文本 | P3 自有 input |

## 待重放

无。旧 vendor 的真实改动都已重放、改由 yttt 承接，或记录在下方的不重放清单里。

## yttt 侧替代

dockerfile、hcl、nix、scss、vue 和 JSX 高亮由 `src/ui/editor/language.rs` 通过 `LanguageRegistry` 注册，查询文件在 `src/ui/editor/queries/`，不再需要 vendor 补丁。

## 已删除

| 补丁 | 原因 |
|---|---|
| `TooltipAppearance` | 主题的 `muted_foreground` 已是同一颜色 |
| Radio 行高和上边距 | Zed 没有对应控件，改用上游默认值 |

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
| 主题字段 `tiles` | 上游 0.6.2 删除了 tiles 画布 |
| 主题字段 `accordion_hover` | 上游已移除，yttt 没有使用 Accordion |
| 其余 32 个文件 | 仅格式差异：`dock/tiles.rs`、`dock/state.rs`、`window_border.rs`、`actions.rs`、`element_ext.rs`、`dialog/*`、`resizable/resize_handle.rs`、`popover.rs`、`breadcrumb.rs`、`clipboard.rs`、`link.rs`、`collapsible.rs`、`searchable_list/adapter.rs`、`button/dropdown_button.rs`、`list/separator_item.rs`、`input/display_map/*`、`input/lsp/*`、`input/popovers/diagnostic_popover.rs`、`input/blink_cursor.rs`、`highlighter/diagnostics.rs`、`setting/fields/*`、`plot/*`、`time/utils.rs`、`text/text_view.rs` |

`crates/assets`、`crates/macros` 没有需要保留的 yttt 改动，由 `gpui-kit-assets` 和 0.8.1 的宏 crate 取代。

## 许可证

`LICENSE-APACHE` 保持原样。yttt 自己新增的 `yttt_ext.rs` 同样按 Apache-2.0 分发。
