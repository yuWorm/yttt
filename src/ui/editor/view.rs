use gpui::{Context, Entity, Styled as _, Window, px, relative};
use gpui_component::input::{Input, InputState, TabSize};

use super::{CodeEditorState, EditorAppearance};

pub fn code_editor_input_state(
    window: &mut Window,
    cx: &mut Context<InputState>,
    editor: &CodeEditorState,
) -> InputState {
    InputState::new(window, cx)
        .placeholder(editor.config().placeholder().to_string())
        .default_value(editor.value().to_string())
        .code_editor(editor.language().to_string())
        .tab_size(TabSize {
            tab_size: editor.config().tab_size(),
            hard_tabs: false,
        })
        .line_number(editor.config().line_number())
        .rows(editor.config().rows())
        .soft_wrap(editor.config().soft_wrap())
        .folding(true)
}

pub fn styled_code_editor_input(
    input: &Entity<InputState>,
    appearance: &EditorAppearance,
) -> Input {
    Input::new(input)
        .appearance(false)
        .text_size(px(appearance.font_size))
        .line_height(relative(appearance.line_height))
        .font_family(appearance.resolved_font_family())
}
