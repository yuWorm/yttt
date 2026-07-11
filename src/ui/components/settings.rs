use super::*;

pub fn workbench_settings_row(
    control_width: Pixels,
    theme: WorkbenchTheme,
    title: impl Into<String>,
    description: impl Into<String>,
    control: AnyElement,
) -> Div {
    let title = title.into();
    let description = description.into();
    let row_style = yttt_row_style(
        YtttRowKind::Settings,
        SelectableState::Inactive,
        true,
        theme,
    );

    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_6()
        .min_h(row_style.height)
        .border_b(row_style.border_width)
        .border_color(row_style.border)
        .bg(row_style.background)
        .py(row_style.padding_y)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(row_style.title)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(row_style.subtitle)
                        .child(description),
                ),
        )
        .child(
            div()
                .flex()
                .justify_end()
                .items_center()
                .w(control_width)
                .flex_none()
                .child(control),
        )
}

pub fn workbench_switch<H>(
    id: impl Into<ElementId>,
    checked: bool,
    theme: WorkbenchTheme,
    on_change: H,
) -> Div
where
    H: Fn(&bool, &mut Window, &mut App) + 'static,
{
    let style = yttt_switch_style(theme);
    let id = id.into();
    let next_checked = !checked;
    let track_background = if checked {
        style.active_background
    } else {
        style.inactive_background
    };
    let border = if checked {
        style.active_border
    } else {
        style.inactive_border
    };
    let thumb = if checked {
        style.active_thumb
    } else {
        style.inactive_thumb
    };

    div()
        .h(style.control_height)
        .flex()
        .items_center()
        .justify_end()
        .child(
            div()
                .id(id)
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .w(style.width)
                .h(style.height)
                .rounded_full()
                .border_2()
                .border_color(border)
                .hover(move |this| this.border_color(style.active_border))
                .on_click(move |_, window, cx| on_change(&next_checked, window, cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .when(checked, |this| this.justify_end())
                        .when(!checked, |this| this.justify_start())
                        .w(style.track_width)
                        .h(style.track_height)
                        .px(style.track_padding)
                        .rounded_full()
                        .border_1()
                        .border_color(border)
                        .bg(track_background)
                        .child(
                            div()
                                .size(style.thumb_size)
                                .rounded_full()
                                .bg(thumb)
                                .shadow_xs(),
                        ),
                ),
        )
}
