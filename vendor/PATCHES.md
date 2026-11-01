# 本地补丁

`vendor/gpui-base` 和 `vendor/gpui-component` 来自 crates.io 的 0.8.1 发布包，经 `Cargo.toml` 的 `[patch.crates-io]` 替换上游。原始内容见提交 `chore: vendor unmodified gpui-base and gpui-component 0.8.1`，用它和当前目录做 diff 就能得到全部本地改动。

- 上游：<https://github.com/longbridge/gpui-kit>，许可证 Apache-2.0，见各目录的 `LICENSE-APACHE`
- 改动过的文件在第一行标注 `Modified for yttt`
- 升级上游时，从新的发布包重新解开，再按本表重放

分类含义见 `docs/design/2026-10-31-gpui-kit-zed-visual-design.md` 第 7 节。

## gpui-base

| 补丁 | 文件 | 分类 | 原因 | 删除条件 |
|---|---|---|---|---|
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
| `TooltipAppearance` | `src/tooltip.rs` | C | Zed 风格的快捷键提示颜色 | P3 用 `gpui-base` 重做 tooltip |
| `Notification::always_show_close_button` | `src/notification.rs` | C | 错误通知要始终显示关闭按钮 | P4 重做通知 |

## 尚未重放

| 补丁 | 现状 | 处理 |
|---|---|---|
| `PopupMenuStyle`、`MenuItemAppearance` | `yttt_popup_menu` 暂时原样返回菜单，当前是上游外观 | 任务 1.3 重放 |
| `hover_matches_selected` | 调用已删除 | 任务 1.5 判断 |
| `flush_search_panel` | 调用已删除 | 任务 1.4 判断 |
| 主题字段 `accordion_hover`、`placeholder_foreground`、`tiles` | 赋值已删除 | 任务 1.5 找到 0.8.1 的对应字段 |
| D 类语言支持 | 未重放，8 个测试失败 | 任务 1.4 |
| 其余差异文件 | 未逐项分类 | 任务 1.2 |

## 许可证

`LICENSE-APACHE` 保持原样。yttt 自己新增的 `yttt_ext.rs` 同样按 Apache-2.0 分发。
