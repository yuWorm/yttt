use std::{ops::Range, sync::Arc};

use gpui::{
    App, Div, HighlightStyle, IntoElement, ParentElement as _, RenderOnce, Rgba, ScrollHandle,
    SharedString, Stateful, Styled as _, StyledText, UniformListScrollHandle, Window, div,
    prelude::*, px, relative, rgba, uniform_list,
};

use crate::ui::theme::EditorTheme;

use super::EditorAppearance;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadonlyCodeRowKind {
    Code,
    Hunk,
    Phantom,
}

#[derive(Clone, Debug)]
pub struct ReadonlyCodeRow {
    pub kind: ReadonlyCodeRowKind,
    pub line_numbers: [Option<usize>; 2],
    pub prefix: SharedString,
    pub content: SharedString,
    pub highlights: Arc<Vec<(Range<usize>, HighlightStyle)>>,
    pub background: Rgba,
    pub foreground: Option<Rgba>,
    pub accent: Rgba,
}

impl ReadonlyCodeRow {
    pub fn code(
        line_numbers: [Option<usize>; 2],
        prefix: impl Into<SharedString>,
        content: impl Into<SharedString>,
        highlights: Arc<Vec<(Range<usize>, HighlightStyle)>>,
        background: Rgba,
        accent: Rgba,
    ) -> Self {
        Self {
            kind: ReadonlyCodeRowKind::Code,
            line_numbers,
            prefix: prefix.into(),
            content: content.into(),
            highlights,
            background,
            foreground: None,
            accent,
        }
    }

    pub fn hunk(content: impl Into<SharedString>, background: Rgba, foreground: Rgba) -> Self {
        Self {
            kind: ReadonlyCodeRowKind::Hunk,
            line_numbers: [None, None],
            prefix: SharedString::default(),
            content: content.into(),
            highlights: Arc::new(Vec::new()),
            background,
            foreground: Some(foreground),
            accent: rgba(0x00000000),
        }
    }

    pub fn phantom(background: Rgba) -> Self {
        Self {
            kind: ReadonlyCodeRowKind::Phantom,
            line_numbers: [None, None],
            prefix: SharedString::default(),
            content: SharedString::default(),
            highlights: Arc::new(Vec::new()),
            background,
            foreground: None,
            accent: rgba(0x00000000),
        }
    }
}

#[derive(IntoElement)]
pub struct ReadonlyCodeView {
    id: SharedString,
    row_debug_prefix: SharedString,
    rows: Arc<Vec<ReadonlyCodeRow>>,
    number_columns: usize,
    content_width: f32,
    vertical_scroll: UniformListScrollHandle,
    horizontal_scroll: ScrollHandle,
    appearance: EditorAppearance,
    theme: EditorTheme,
    border: Rgba,
}

impl ReadonlyCodeView {
    pub fn new(
        id: impl Into<SharedString>,
        rows: Arc<Vec<ReadonlyCodeRow>>,
        vertical_scroll: UniformListScrollHandle,
        horizontal_scroll: ScrollHandle,
        appearance: EditorAppearance,
        theme: EditorTheme,
        border: Rgba,
    ) -> Self {
        let id = id.into();
        Self {
            row_debug_prefix: id.clone(),
            id,
            rows,
            number_columns: 1,
            content_width: 900.0,
            vertical_scroll,
            horizontal_scroll,
            appearance,
            theme,
            border,
        }
    }

    pub fn number_columns(mut self, columns: usize) -> Self {
        self.number_columns = columns.min(2);
        self
    }

    pub fn content_width(mut self, width: f32) -> Self {
        self.content_width = width;
        self
    }

    pub fn row_debug_prefix(mut self, prefix: impl Into<SharedString>) -> Self {
        self.row_debug_prefix = prefix.into();
        self
    }
}

impl RenderOnce for ReadonlyCodeView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let row_count = self.rows.len();
        let rows = self.rows.clone();
        let appearance = self.appearance.clone();
        let theme = self.theme;
        let border = self.border;
        let number_columns = self.number_columns;
        let row_debug_prefix = self.row_debug_prefix.clone();
        let horizontal_id: SharedString = format!("{}-horizontal-scroll", self.id).into();
        let horizontal_scroll = self.horizontal_scroll.clone();
        let horizontal_scroll_for_wheel = horizontal_scroll.clone();
        let vertical_scroll = self.vertical_scroll.clone();
        let vertical_scroll_for_wheel = vertical_scroll.clone();

        div()
            .id(horizontal_id.clone())
            .debug_selector(move || horizontal_id.to_string())
            .size_full()
            .relative()
            .min_w_0()
            .overflow_x_scroll()
            .track_scroll(&horizontal_scroll)
            .bg(theme.background)
            .child(
                uniform_list(self.id, row_count, move |range, _window, _cx| {
                    range
                        .filter_map(|index| {
                            rows.get(index).map(|row| {
                                render_readonly_code_row(
                                    index,
                                    row,
                                    number_columns,
                                    &row_debug_prefix,
                                    &appearance,
                                    theme,
                                    border,
                                )
                            })
                        })
                        .collect()
                })
                .h_full()
                .min_w_full()
                .w(px(self.content_width))
                .track_scroll(&vertical_scroll),
            )
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .on_scroll_wheel(move |event, window, cx| {
                        let delta = event.delta.pixel_delta(window.line_height());
                        let (delta_x, delta_y) = if delta.x != px(0.0) && delta.y != px(0.0) {
                            if delta.x.abs() > delta.y.abs() {
                                (delta.x, px(0.0))
                            } else {
                                (px(0.0), delta.y)
                            }
                        } else {
                            (delta.x, delta.y)
                        };
                        let mut changed = false;
                        if delta_x != px(0.0) {
                            let mut offset = horizontal_scroll_for_wheel.offset();
                            let old_offset = offset;
                            offset.x += delta_x;
                            horizontal_scroll_for_wheel.set_offset(offset);
                            changed |= offset != old_offset;
                        }
                        if delta_y != px(0.0) {
                            let base_handle =
                                vertical_scroll_for_wheel.0.borrow().base_handle.clone();
                            let mut offset = base_handle.offset();
                            let old_offset = offset;
                            offset.x = px(0.0);
                            offset.y += delta_y;
                            base_handle.set_offset(offset);
                            changed |= offset != old_offset;
                        }
                        if changed {
                            window.refresh();
                        }
                        cx.stop_propagation();
                    }),
            )
    }
}

fn render_readonly_code_row(
    index: usize,
    row: &ReadonlyCodeRow,
    number_columns: usize,
    debug_prefix: &SharedString,
    appearance: &EditorAppearance,
    theme: EditorTheme,
    border: Rgba,
) -> Stateful<Div> {
    let row_height =
        px((appearance.font_size * appearance.line_height).max(appearance.font_size + 2.0));
    let foreground = row.foreground.unwrap_or(theme.foreground);
    let debug_selector = format!("{debug_prefix}-{index}");
    let mut element = editor_text_div(div(), appearance)
        .id((debug_prefix.clone(), index))
        .debug_selector(move || debug_selector.clone())
        .flex()
        .items_center()
        .h(row_height)
        .min_w_full()
        .bg(row.background)
        .when(row.kind == ReadonlyCodeRowKind::Hunk, |this| {
            this.px_5().border_y_1().border_color(border)
        });

    if row.kind != ReadonlyCodeRowKind::Hunk {
        element = element.child(div().w(px(3.0)).h_full().flex_none().bg(row.accent));
        if appearance.line_numbers {
            for line_number in row.line_numbers.iter().take(number_columns) {
                element = element.child(
                    div()
                        .w(px((appearance.font_size * 3.7).max(44.0)))
                        .pr_2()
                        .flex_none()
                        .text_right()
                        .text_color(theme.line_number)
                        .child(line_number.map(|line| line.to_string()).unwrap_or_default()),
                );
            }
        }
        element = element.child(
            div()
                .w(px(22.0))
                .flex_none()
                .text_center()
                .text_color(row.accent)
                .child(row.prefix.clone()),
        );
    }

    element.child(
        div()
            .flex_1()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_color(foreground)
            .child(
                StyledText::new(row.content.clone())
                    .with_highlights(row.highlights.as_ref().clone()),
            ),
    )
}

fn editor_text_div(element: Div, appearance: &EditorAppearance) -> Div {
    element
        .text_size(px(appearance.font_size))
        .line_height(relative(appearance.line_height))
        .font_family(appearance.resolved_font_family())
}
