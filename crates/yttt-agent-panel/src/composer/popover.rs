use gpui::{
    Context, IntoElement, MouseButton, ParentElement as _, SharedString, Styled as _, div,
    prelude::*, px, relative,
};
use gpui_component::{h_flex, v_flex};
use yttt_ui::{
    Icon, IconName, SelectableState,
    primitives::row::{YtttRowKind, yttt_row},
};

use super::{Composer, Popover, trigger::TriggerKind};

impl Composer {
    /// Opens above the prompt box, full width, so it never covers what the
    /// user is typing.
    pub(super) fn render_popover(
        &self,
        popover: &Popover,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = self.theme;
        let ui_style = self.ui_style;
        let icon = match popover.trigger.kind {
            TriggerKind::Mention => IconName::FileText,
            TriggerKind::Command => IconName::SquareTerminal,
        };
        let prefix = match popover.trigger.kind {
            TriggerKind::Mention => "",
            TriggerKind::Command => "/",
        };
        v_flex()
            .absolute()
            .left_0()
            .right_0()
            .bottom(relative(1.))
            .mb(ui_style.spacing.xs)
            .p(ui_style.spacing.xs)
            .gap(px(1.))
            .rounded(ui_style.radius.surface)
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface_elevated)
            .shadow_md()
            .children(
                popover
                    .candidates
                    .iter()
                    .enumerate()
                    .map(|(ix, candidate)| {
                        let state = if ix == popover.selected {
                            SelectableState::Active
                        } else {
                            SelectableState::Inactive
                        };
                        yttt_row(YtttRowKind::PaletteCompact, state, true, theme, ui_style)
                            .id(SharedString::from(format!("composer-candidate-{ix}")))
                            .flex()
                            .items_center()
                            .gap(ui_style.spacing.sm)
                            .cursor_pointer()
                            .text_size(ui_style.text.small)
                            .child(
                                Icon::new(icon.clone())
                                    .size(px(14.))
                                    .text_color(theme.icon_muted),
                            )
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(ui_style.spacing.sm)
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_color(theme.text)
                                            .child(format!("{prefix}{}", candidate.label)),
                                    )
                                    .when_some(candidate.detail.clone(), |this, detail| {
                                        this.child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .text_color(theme.text_subtle)
                                                .child(detail),
                                        )
                                    }),
                            )
                            // Mouse down, not click, so the prompt box keeps focus.
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, window, cx| {
                                    this.accept(ix, window, cx);
                                    cx.stop_propagation();
                                }),
                            )
                    }),
            )
    }
}
