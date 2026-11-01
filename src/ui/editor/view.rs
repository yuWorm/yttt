use gpui::{Context, Entity, Styled as _, Window, px, relative};
use gpui_component::input::{Editor, EditorState, TabSize};
use yttt_ui::primitives::input::yttt_borderless_editor;

use super::{CodeEditorState, EditorAppearance};

pub fn code_editor_input_state(
    window: &mut Window,
    cx: &mut Context<EditorState>,
    editor: &CodeEditorState,
) -> EditorState {
    EditorState::new(window, cx)
        .placeholder(editor.config().placeholder().to_string())
        .default_value(editor.value().to_string())
        .language(editor.language().to_string())
        .tab_size(TabSize {
            tab_size: editor.config().tab_size(),
            hard_tabs: false,
        })
        .line_number(editor.config().line_number())
        .soft_wrap(editor.config().soft_wrap())
        .folding(true)
}

pub fn styled_code_editor_input(
    input: &Entity<EditorState>,
    appearance: &EditorAppearance,
) -> Editor {
    yttt_borderless_editor(input)
        .text_size(px(appearance.font_size))
        .line_height(relative(appearance.line_height))
        .font_family(appearance.resolved_font_family())
}
