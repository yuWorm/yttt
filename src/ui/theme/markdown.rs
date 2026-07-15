use gpui::{Hsla, Rgba};
use gpui_component::ThemeMode;
use gpui_markdown_editor::MarkdownEditorTheme;

use super::ThemeRuntime;

fn color(value: Rgba) -> Hsla {
    value.into()
}

impl ThemeRuntime {
    /// Builds the complete, instance-local theme consumed by the Markdown editor.
    ///
    /// The component intentionally has its own semantic token schema. Mapping it
    /// here keeps Markdown documents on the same resolved palette as the rest of
    /// the workbench, including imported themes and window-material opacity.
    pub fn to_markdown_editor_theme(
        &self,
        editor_font_size: f32,
        editor_line_height: f32,
    ) -> MarkdownEditorTheme {
        let mut theme = if self.mode == ThemeMode::Light {
            MarkdownEditorTheme::light_theme()
        } else {
            MarkdownEditorTheme::default_theme()
        };
        theme.name = format!("{} Markdown", self.theme_name);

        let ui = self.ui;
        let editor = self.editor;
        let colors = &mut theme.colors;
        colors.editor_background = color(editor.background);
        colors.source_mode_block_bg = color(editor.active_line);
        colors.comment_bg = color(ui.warning.alpha(0.14));
        colors.text_default = color(editor.foreground);
        colors.text_link = color(ui.accent);
        colors.text_placeholder = color(ui.text_subtle);
        colors.text_h1 = color(editor.foreground);
        colors.text_h2 = color(editor.foreground);
        colors.text_h3 = color(editor.foreground);
        colors.text_h4 = color(editor.foreground);
        colors.text_h5 = color(editor.foreground);
        colors.text_h6 = color(ui.text_muted);
        colors.border_h1 = color(ui.border_strong);
        colors.border_h2 = color(ui.border);
        colors.text_quote = color(ui.text_muted);
        colors.border_quote = color(ui.border_strong);

        colors.callout_note_bg = color(ui.accent.alpha(0.12));
        colors.callout_note_border = color(ui.accent);
        colors.callout_tip_bg = color(ui.success.alpha(0.12));
        colors.callout_tip_border = color(ui.success);
        colors.callout_important_bg = color(ui.accent.alpha(0.16));
        colors.callout_important_border = color(ui.accent);
        colors.callout_warning_bg = color(ui.warning.alpha(0.14));
        colors.callout_warning_border = color(ui.warning);
        colors.callout_caution_bg = color(ui.danger.alpha(0.14));
        colors.callout_caution_border = color(ui.danger);

        colors.footnote_bg = color(ui.surface_elevated);
        colors.footnote_border = color(ui.border);
        colors.footnote_badge_bg = color(ui.active_surface);
        colors.footnote_badge_text = color(ui.text);
        colors.footnote_backref = color(ui.accent);
        colors.task_checkbox_border = color(ui.border_strong);
        colors.task_checkbox_bg = color(ui.surface);
        colors.task_checkbox_checked_bg = color(ui.accent);
        colors.task_checkbox_check = color(editor.background.alpha(1.0));
        colors.separator_color = color(ui.border);

        colors.code_bg = color(ui.surface_elevated);
        colors.code_text = color(editor.foreground);
        colors.code_language_input_bg = color(ui.surface);
        colors.code_language_input_border = color(ui.border_strong);
        colors.code_language_input_text = color(ui.text);
        colors.code_language_input_placeholder = color(ui.text_subtle);
        colors.code_syntax_comment = color(editor.syntax.comment);
        colors.code_syntax_keyword = color(editor.syntax.keyword);
        colors.code_syntax_string = color(editor.syntax.string);
        colors.code_syntax_number = color(editor.syntax.number);
        colors.code_syntax_type = color(editor.syntax.type_);
        colors.code_syntax_function = color(editor.syntax.function);
        colors.code_syntax_constant = color(editor.syntax.constant);
        colors.code_syntax_variable = color(editor.syntax.variable);
        colors.code_syntax_property = color(editor.syntax.property);
        colors.code_syntax_operator = color(editor.syntax.operator);
        colors.code_syntax_punctuation = color(editor.syntax.punctuation);

        colors.table_border = color(ui.border);
        colors.table_header_bg = color(ui.surface_elevated);
        colors.table_cell_bg = color(editor.background);
        colors.table_cell_active_outline = color(ui.focus_ring);
        colors.table_axis_preview_bg = color(ui.hover_surface);
        colors.table_axis_selected_bg = color(ui.selection);
        colors.table_append_button_bg = color(ui.surface_elevated);
        colors.table_append_button_hover = color(ui.hover_surface);
        colors.table_append_button_text = color(ui.text_muted);
        colors.image_placeholder_bg = color(ui.surface_elevated);
        colors.image_placeholder_border = color(ui.border);
        colors.image_placeholder_text = color(ui.text_muted);
        colors.image_caption_text = color(ui.text_muted);
        colors.scrollbar_thumb = color(ui.text_subtle.alpha(0.72));
        colors.cursor = color(ui.accent);
        colors.selection = color(ui.selection);

        colors.dialog_backdrop = color(self.window_material.scrim);
        colors.dialog_surface = color(ui.surface_elevated);
        colors.dialog_border = color(ui.border);
        colors.dialog_title = color(ui.text);
        colors.dialog_body = color(ui.text_muted);
        colors.dialog_muted = color(ui.text_subtle);
        colors.dialog_primary_button_bg = color(ui.active_surface);
        colors.dialog_primary_button_hover = color(ui.hover_surface);
        colors.dialog_primary_button_text = color(ui.text);
        colors.dialog_secondary_button_bg = color(ui.surface);
        colors.dialog_secondary_button_hover = color(ui.hover_surface);
        colors.dialog_secondary_button_text = color(ui.text_muted);
        colors.dialog_danger_button_bg = color(ui.danger);
        colors.dialog_danger_button_hover = color(ui.danger.alpha(0.86));
        colors.dialog_danger_button_text = color(editor.background.alpha(1.0));
        colors.status_bar_background = color(ui.tabbar_background);
        colors.status_bar_text = color(ui.text);
        colors.status_bar_text_dim = color(ui.text_muted);
        colors.status_bar_button_hover = color(ui.hover_surface);

        let font_size = editor_font_size.max(10.0);
        let typography = &mut theme.typography;
        typography.text_size = font_size;
        typography.text_line_height = editor_line_height.max(1.0);
        typography.h1_size = font_size * 2.0;
        typography.h2_size = font_size * 1.5;
        typography.h3_size = font_size * 1.25;
        typography.h4_size = font_size * 1.1;
        typography.h5_size = font_size;
        typography.h6_size = font_size * 0.9;
        typography.code_size = font_size;

        let dimensions = &mut theme.dimensions;
        dimensions.editor_padding = 20.0;
        dimensions.block_gap = 6.0;
        dimensions.centered_shrink_start = 900.0;
        dimensions.centered_shrink_end = 1_600.0;
        dimensions.centered_min_ratio = 0.72;
        theme
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_theme_uses_resolved_workbench_palette_and_editor_metrics() {
        let runtime = ThemeRuntime::default();
        let theme = runtime.to_markdown_editor_theme(15.0, 1.6);

        assert_eq!(
            theme.colors.editor_background,
            color(runtime.editor.background)
        );
        assert_eq!(theme.colors.text_default, color(runtime.editor.foreground));
        assert_eq!(theme.colors.text_link, color(runtime.ui.accent));
        assert_eq!(theme.colors.selection, color(runtime.ui.selection));
        assert_eq!(theme.typography.text_size, 15.0);
        assert_eq!(theme.typography.text_line_height, 1.6);
        assert_eq!(theme.typography.code_size, 15.0);
    }
}
