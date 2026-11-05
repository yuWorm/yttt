//! The composer's chrome around the text: attachments above it, and below it
//! the mode and model pickers and the context meter.
use gpui::{
    AnyElement, Context, IntoElement, MouseButton, ParentElement as _, SharedString,
    StatefulInteractiveElement as _, Styled as _, div, prelude::*, px, relative,
};
use gpui_component::{h_flex, v_flex};
use yttt_ui::{
    Icon, IconName, SelectableState,
    primitives::row::{YtttRowKind, yttt_row},
};

use super::{Composer, ComposerEvent, OpenMenu, SelectorKind};
use crate::thread_view::render_attachment_preview;

/// The meter turns to warning, then danger, as context runs out.
const USAGE_WARNING: f64 = 0.75;
const USAGE_DANGER: f64 = 0.9;

impl Composer {
    pub(super) fn render_attachments(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        // Images sent to a model without vision would be ignored.
        let unseen = self
            .attachments
            .iter()
            .any(|attachment| attachment.kind.is_image())
            .then(|| self.catalog.active())
            .flatten()
            .filter(|model| !model.capabilities.vision)
            .map(|model| format!("{} can't see images", model.name));
        h_flex()
            .flex_wrap()
            .gap(self.ui_style.spacing.sm)
            .px(self.ui_style.spacing.md)
            .pt(self.ui_style.spacing.sm)
            .children(self.attachments.iter().enumerate().map(|(ix, attachment)| {
                let group = SharedString::from(format!("attachment-{ix}"));
                div()
                    .group(group.clone())
                    .relative()
                    .child(render_attachment_preview(attachment, theme, self.ui_style))
                    .child(
                        div()
                            .id(SharedString::from(format!("attachment-remove-{ix}")))
                            .absolute()
                            .top(px(-6.))
                            .right(px(-6.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .size(px(16.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.surface_elevated)
                            .cursor_pointer()
                            .invisible()
                            .group_hover(group, |this| this.visible())
                            .child(
                                Icon::new(IconName::Close)
                                    .size(px(10.))
                                    .text_color(theme.icon),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if ix < this.attachments.len() {
                                    this.attachments.remove(ix);
                                }
                                cx.notify();
                            })),
                    )
            }))
            .when_some(unseen, |this, warning| {
                this.child(
                    h_flex()
                        .gap(px(4.))
                        .text_size(self.ui_style.text.xsmall)
                        .text_color(theme.warning)
                        .child(Icon::new(IconName::TriangleAlert).size(px(11.)))
                        .child(warning),
                )
            })
    }

    /// The picker's current choice; clicking opens its menu.
    pub(super) fn render_selector_button(
        &self,
        kind: SelectorKind,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let selector = self.selector(kind);
        let option = selector.options.get(selector.selected)?;
        let theme = self.theme;
        let open = self.open_menu == Some(OpenMenu::Selector(kind));
        Some(
            h_flex()
                .id(match kind {
                    SelectorKind::Mode => "composer-mode",
                })
                .min_w_0()
                .h(px(24.))
                .gap(px(3.))
                .px(px(6.))
                .rounded(px(5.))
                .text_size(self.ui_style.text.xsmall)
                .text_color(theme.text_muted)
                .cursor_pointer()
                .when(open, |this| this.bg(theme.element_active))
                .hover(move |this| this.bg(theme.ghost_element_hover).text_color(theme.text))
                .child(div().truncate().child(option.label.clone()))
                .child(
                    Icon::new(IconName::ChevronDown)
                        .size(px(11.))
                        .text_color(theme.icon_muted),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.open_menu = if this.open_menu == Some(OpenMenu::Selector(kind)) {
                        None
                    } else {
                        Some(OpenMenu::Selector(kind))
                    };
                    this.popover = None;
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    /// The open picker's choices, above the prompt box like the mention
    /// popover.
    pub(super) fn render_menu(
        &self,
        kind: SelectorKind,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = self.theme;
        let ui_style = self.ui_style;
        let selector = self.selector(kind);
        v_flex()
            .absolute()
            .left_0()
            .bottom(relative(1.))
            .mb(ui_style.spacing.xs)
            .min_w(px(240.))
            .p(ui_style.spacing.xs)
            .gap(px(1.))
            .rounded(ui_style.radius.surface)
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface_elevated)
            .shadow_md()
            .children(selector.options.iter().enumerate().map(|(ix, option)| {
                let selected = ix == selector.selected;
                let id = option.id.clone();
                yttt_row(
                    YtttRowKind::PaletteCompact,
                    if selected {
                        SelectableState::Active
                    } else {
                        SelectableState::Inactive
                    },
                    true,
                    theme,
                    ui_style,
                )
                .id(SharedString::from(format!("composer-menu-{ix}")))
                .flex()
                .items_center()
                .gap(ui_style.spacing.sm)
                .cursor_pointer()
                .text_size(ui_style.text.small)
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().text_color(theme.text).child(option.label.clone()))
                        .when_some(option.detail.clone(), |this, detail| {
                            this.child(
                                div()
                                    .truncate()
                                    .text_size(ui_style.text.xsmall)
                                    .text_color(theme.text_subtle)
                                    .child(detail),
                            )
                        }),
                )
                .when(selected, |this| {
                    this.child(
                        Icon::new(IconName::Check)
                            .size(px(13.))
                            .text_color(theme.accent),
                    )
                })
                // Mouse down, not click, so the prompt box keeps focus.
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.selector_mut(kind).selected = ix;
                        this.open_menu = None;
                        cx.emit(ComposerEvent::Selected {
                            kind,
                            id: id.clone(),
                        });
                        cx.notify();
                        cx.stop_propagation();
                    }),
                )
            }))
    }

    /// One round button that sends, stops a busy turn when there is nothing
    /// to send, and rests greyed out when there is neither.
    pub(super) fn render_send_button(
        &self,
        busy: bool,
        empty: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        const SIZE: f32 = 28.;
        let theme = self.theme;
        let (icon, background, hover, foreground, enabled) = match (busy, empty) {
            (true, true) => (
                IconName::Square,
                theme.element_background,
                theme.element_hover,
                theme.text,
                true,
            ),
            (false, true) => (
                IconName::ArrowUp,
                theme.element_background,
                theme.element_background,
                theme.text_disabled,
                false,
            ),
            (_, false) => (
                IconName::ArrowUp,
                theme.accent,
                theme.accent.alpha(0.85),
                theme.app_background,
                true,
            ),
        };
        div()
            .id("composer-send")
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .size(px(SIZE))
            .rounded(px(SIZE / 2.))
            .bg(background)
            .when(enabled, |this| {
                this.cursor_pointer()
                    .hover(move |this| this.bg(hover))
                    .active(|this| this.opacity(0.8))
            })
            .child(Icon::new(icon).size(px(14.)).text_color(foreground))
            .on_click(cx.listener(Composer::on_send_or_stop))
    }

    /// Context used against the window, as text and a thin bar.
    pub(super) fn render_usage(&self) -> Option<AnyElement> {
        let usage = self.usage?;
        if usage.limit_tokens == 0 {
            return None;
        }
        let theme = self.theme;
        let ratio = (usage.used_tokens as f64 / usage.limit_tokens as f64).min(1.);
        let color = if ratio >= USAGE_DANGER {
            theme.danger
        } else if ratio >= USAGE_WARNING {
            theme.warning
        } else {
            theme.text_subtle
        };
        const BAR_WIDTH: f32 = 32.;
        Some(
            h_flex()
                .flex_shrink_0()
                .gap(px(6.))
                .mr(px(4.))
                .text_size(self.ui_style.text.xsmall)
                .text_color(color)
                .child(format!(
                    "{} / {}",
                    format_tokens(usage.used_tokens),
                    format_tokens(usage.limit_tokens)
                ))
                .child(
                    div()
                        .w(px(BAR_WIDTH))
                        .h(px(4.))
                        .rounded(px(2.))
                        .bg(theme.element_background)
                        .child(
                            div()
                                .h_full()
                                .w(px(BAR_WIDTH * ratio as f32))
                                .rounded(px(2.))
                                .bg(if ratio >= USAGE_WARNING {
                                    color
                                } else {
                                    theme.accent
                                }),
                        ),
                )
                .into_any_element(),
        )
    }
}

/// `950`, `4.2k`, `200k`, `1M`, `1.5M`.
pub(crate) fn format_tokens(tokens: u64) -> String {
    match tokens {
        0..1_000 => tokens.to_string(),
        1_000..10_000 => format!("{:.1}k", tokens as f64 / 1_000.),
        10_000..1_000_000 => format!("{}k", tokens / 1_000),
        _ => {
            let millions = tokens as f64 / 1_000_000.;
            if millions.fract() == 0. {
                format!("{millions}M")
            } else {
                format!("{millions:.1}M")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::format_tokens;

    #[test]
    fn tokens_read_compactly() {
        assert_eq!(format_tokens(950), "950");
        assert_eq!(format_tokens(12_400), "12k");
        assert_eq!(format_tokens(4_250), "4.2k");
        assert_eq!(format_tokens(200_000), "200k");
        assert_eq!(format_tokens(1_000_000), "1M");
        assert_eq!(format_tokens(1_500_000), "1.5M");
    }
}
