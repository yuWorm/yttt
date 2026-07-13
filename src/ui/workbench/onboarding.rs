use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum OnboardingStep {
    #[default]
    Layout,
    Agent,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct OnboardingState {
    pub step: OnboardingStep,
    pub selected_layout: DefaultLayoutKind,
    pub selected_agent: BuiltinAgent,
}

pub(super) fn onboarding_view(
    cx: &mut Context<WorkbenchView>,
    state: OnboardingState,
    ui_text: &UiText,
    theme: WorkbenchTheme,
    command_palette_keybinding: Option<String>,
) -> Div {
    let step = match state.step {
        OnboardingStep::Layout => layout_step(cx, state, ui_text, theme),
        OnboardingStep::Agent => agent_step(cx, state, ui_text, theme),
    };

    div()
        .flex()
        .flex_1()
        .w_full()
        .min_h_0()
        .items_center()
        .justify_center()
        .bg(theme.app_background)
        .px_8()
        .py_6()
        .child(
            div()
                .flex()
                .w_full()
                .max_w(px(720.0))
                .flex_col()
                .gap_5()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_2()
                        .text_center()
                        .child(
                            div()
                                .text_2xl()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.text)
                                .child(ui_text.get(UiTextKey::OnboardingTitle)),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.text_muted)
                                .child(ui_text.get(UiTextKey::OnboardingSubtitle)),
                        ),
                )
                .child(step)
                .child(command_palette_hint(
                    cx,
                    ui_text,
                    theme,
                    command_palette_keybinding,
                )),
        )
}

fn layout_step(
    cx: &mut Context<WorkbenchView>,
    state: OnboardingState,
    ui_text: &UiText,
    theme: WorkbenchTheme,
) -> Div {
    let mut layout_picker = div().flex().w_full().gap_3();
    for (index, layout_kind) in DefaultLayoutKind::ALL.into_iter().enumerate() {
        let selected = state.selected_layout == layout_kind;
        let (title, description) = layout_text(layout_kind, ui_text);
        layout_picker = layout_picker.child(
            div()
                .id(("onboarding-layout", index))
                .debug_selector(move || format!("onboarding-layout-{}", layout_kind.id()))
                .cursor_pointer()
                .flex()
                .flex_1()
                .min_w_0()
                .flex_col()
                .gap_3()
                .rounded_lg()
                .border(if selected { px(2.0) } else { px(1.0) })
                .border_color(if selected { theme.accent } else { theme.border })
                .bg(if selected {
                    theme.active_surface
                } else {
                    theme.surface_elevated
                })
                .p_4()
                .hover(move |this| this.bg(theme.hover_surface))
                .on_click(cx.listener(move |this, _, _window, cx| {
                    this.select_onboarding_layout(layout_kind);
                    cx.notify();
                }))
                .child(layout_preview(
                    layout_kind,
                    state.selected_agent,
                    ui_text,
                    theme,
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.text)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.text_muted)
                                .child(description),
                        ),
                ),
        );
    }

    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text_muted)
                .child(ui_text.get(UiTextKey::OnboardingLayoutHeading)),
        )
        .child(layout_picker)
        .child(
            div().flex().justify_end().child(
                yttt_button(
                    "onboarding-next",
                    ui_text.get(UiTextKey::OnboardingNext),
                    YtttButtonVariant::Primary,
                    theme,
                    cx,
                )
                .debug_selector(|| "onboarding-next".to_string())
                .on_click(cx.listener(|this, _, _window, cx| {
                    this.advance_onboarding();
                    cx.notify();
                })),
            ),
        )
}

fn agent_step(
    cx: &mut Context<WorkbenchView>,
    state: OnboardingState,
    ui_text: &UiText,
    theme: WorkbenchTheme,
) -> Div {
    let mut agent_picker = div().flex().w_full().gap_2();
    for (index, agent) in BuiltinAgent::ALL.into_iter().enumerate() {
        let selected = state.selected_agent == agent;
        agent_picker = agent_picker.child(
            div()
                .id(("onboarding-agent", index))
                .debug_selector(move || format!("onboarding-agent-{}", agent.id()))
                .cursor_pointer()
                .flex()
                .flex_1()
                .min_w_0()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .h(px(88.0))
                .rounded_lg()
                .border(if selected { px(2.0) } else { px(1.0) })
                .border_color(if selected { theme.accent } else { theme.border })
                .bg(if selected {
                    theme.active_surface
                } else {
                    theme.surface_elevated
                })
                .hover(move |this| this.bg(theme.hover_surface))
                .on_click(cx.listener(move |this, _, _window, cx| {
                    this.select_onboarding_agent(agent);
                    cx.notify();
                }))
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if selected {
                            theme.text
                        } else {
                            theme.text_muted
                        })
                        .truncate()
                        .child(agent.display_name()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_subtle)
                        .child(agent.command()),
                ),
        );
    }

    div()
        .flex()
        .flex_col()
        .gap_4()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text_muted)
                        .child(ui_text.get(UiTextKey::OnboardingAgentHeading)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_subtle)
                        .child(ui_text.get(UiTextKey::OnboardingAgentSubtitle)),
                ),
        )
        .child(agent_picker)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text_muted)
                        .child(ui_text.get(UiTextKey::OnboardingLayoutHeading)),
                )
                .child(layout_preview(
                    state.selected_layout,
                    state.selected_agent,
                    ui_text,
                    theme,
                )),
        )
        .child(
            div()
                .flex()
                .justify_between()
                .child(
                    yttt_button(
                        "onboarding-back",
                        ui_text.get(UiTextKey::OnboardingBack),
                        YtttButtonVariant::Secondary,
                        theme,
                        cx,
                    )
                    .debug_selector(|| "onboarding-back".to_string())
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.return_to_onboarding_layout();
                        cx.notify();
                    })),
                )
                .child(
                    yttt_button(
                        "onboarding-complete",
                        ui_text.get(UiTextKey::OnboardingContinue),
                        YtttButtonVariant::Primary,
                        theme,
                        cx,
                    )
                    .debug_selector(|| "onboarding-complete".to_string())
                    .on_click(cx.listener(|this, _, _window, cx| {
                        if let Err(error) = this.complete_onboarding() {
                            this.load_error = Some(error);
                        }
                        cx.notify();
                    })),
                ),
        )
}

fn layout_text(layout_kind: DefaultLayoutKind, ui_text: &UiText) -> (&'static str, &'static str) {
    match layout_kind {
        DefaultLayoutKind::SplitPane => (
            ui_text.get(UiTextKey::OnboardingSplitLayoutTitle),
            ui_text.get(UiTextKey::OnboardingSplitLayoutDescription),
        ),
        DefaultLayoutKind::SeparateTabs => (
            ui_text.get(UiTextKey::OnboardingTabsLayoutTitle),
            ui_text.get(UiTextKey::OnboardingTabsLayoutDescription),
        ),
    }
}

fn layout_preview(
    layout_kind: DefaultLayoutKind,
    selected_agent: BuiltinAgent,
    ui_text: &UiText,
    theme: WorkbenchTheme,
) -> Div {
    match layout_kind {
        DefaultLayoutKind::SplitPane => div()
            .flex()
            .h(px(72.0))
            .w_full()
            .overflow_hidden()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface_elevated)
            .child(
                div()
                    .flex()
                    .flex_basis(relative(0.65))
                    .items_center()
                    .justify_center()
                    .border_r_1()
                    .border_color(theme.border)
                    .text_xs()
                    .text_color(theme.text)
                    .child(selected_agent.display_name()),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(ui_text.get(UiTextKey::OnboardingShellPane)),
            ),
        DefaultLayoutKind::SeparateTabs => div()
            .flex()
            .h(px(72.0))
            .w_full()
            .flex_col()
            .overflow_hidden()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface_elevated)
            .child(
                div()
                    .flex()
                    .h(px(26.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .border_r_1()
                            .border_color(theme.border)
                            .bg(theme.active_surface)
                            .px_3()
                            .text_xs()
                            .text_color(theme.text)
                            .child(ui_text.get(UiTextKey::OnboardingAgentPane)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .px_3()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child(ui_text.get(UiTextKey::OnboardingShellPane)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(selected_agent.display_name()),
            ),
    }
}

fn command_palette_hint(
    cx: &mut Context<WorkbenchView>,
    ui_text: &UiText,
    theme: WorkbenchTheme,
    command_palette_keybinding: Option<String>,
) -> Div {
    let mut hint = div()
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .text_xs()
        .text_color(theme.text_muted)
        .child(ui_text.get(UiTextKey::OnboardingCommandPaletteHint));
    if let Some(keybinding) = command_palette_keybinding {
        hint = hint.child(workbench_keybinding_badge(keybinding, theme));
    }
    hint.child(
        yttt_button(
            "onboarding-open-command-palette",
            ui_text.get(UiTextKey::CommandPalette),
            YtttButtonVariant::Ghost,
            theme,
            cx,
        )
        .debug_selector(|| "onboarding-open-command-palette".to_string())
        .on_click(cx.listener(|this, _, window, cx| {
            this.on_open_command_palette(&OpenCommandPalette, window, cx);
        })),
    )
}
