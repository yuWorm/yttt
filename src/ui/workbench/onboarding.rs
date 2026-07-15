use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum OnboardingStep {
    #[default]
    Language,
    Layout,
    Agent,
    ZedImport,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OnboardingState {
    pub step: OnboardingStep,
    pub selected_language: LanguageSetting,
    pub selected_layout: DefaultLayoutKind,
    pub selected_agent: BuiltinAgent,
    pub zed_detection: ZedThemeDetection,
    pub zed_import_completed: bool,
}

impl OnboardingState {
    pub fn new(zed_detection: ZedThemeDetection, selected_language: LanguageSetting) -> Self {
        Self {
            step: OnboardingStep::Language,
            selected_language: match selected_language {
                LanguageSetting::System => LanguageSetting::English,
                language => language,
            },
            selected_layout: DefaultLayoutKind::default(),
            selected_agent: BuiltinAgent::default(),
            zed_detection,
            zed_import_completed: false,
        }
    }
}

impl Default for OnboardingState {
    fn default() -> Self {
        Self::new(ZedThemeDetection::default(), LanguageSetting::English)
    }
}

pub(super) fn onboarding_view(
    cx: &mut Context<WorkbenchView>,
    state: &OnboardingState,
    ui_text: &UiText,
    theme: WorkbenchTheme,
    command_palette_keybinding: Option<String>,
) -> Div {
    let step = match state.step {
        OnboardingStep::Language => language_step(cx, state, ui_text, theme),
        OnboardingStep::Layout => layout_step(cx, state, ui_text, theme),
        OnboardingStep::Agent => agent_step(cx, state, ui_text, theme),
        OnboardingStep::ZedImport => zed_import_step(cx, state, ui_text, theme),
    };

    div()
        .flex()
        .flex_1()
        .w_full()
        .min_h_0()
        .items_center()
        .justify_center()
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

fn language_step(
    cx: &mut Context<WorkbenchView>,
    state: &OnboardingState,
    ui_text: &UiText,
    theme: WorkbenchTheme,
) -> Div {
    let mut language_picker = div().flex().w_full().gap_3();
    for (index, language) in [LanguageSetting::English, LanguageSetting::Chinese]
        .into_iter()
        .enumerate()
    {
        let selected = state.selected_language == language;
        let language_id = match language {
            LanguageSetting::Chinese => "zh-cn",
            LanguageSetting::System | LanguageSetting::English => "en",
        };
        language_picker = language_picker.child(
            div()
                .id(("onboarding-language", index))
                .debug_selector(move || format!("onboarding-language-{language_id}"))
                .cursor_pointer()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .h(rems(5.5))
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
                    if let Err(error) = this.select_onboarding_language(language) {
                        this.load_error = Some(error.to_string());
                    }
                    cx.notify();
                }))
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if selected {
                            theme.text
                        } else {
                            theme.text_muted
                        })
                        .child(language_setting_label(language)),
                ),
        );
    }

    div()
        .debug_selector(|| "onboarding-language-step".to_string())
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
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child(ui_text.get(UiTextKey::OnboardingLanguageHeading)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_muted)
                        .child(ui_text.get(UiTextKey::OnboardingLanguageSubtitle)),
                ),
        )
        .child(language_picker)
        .child(
            div().flex().justify_end().child(
                yttt_button(
                    "onboarding-language-next",
                    ui_text.get(UiTextKey::OnboardingNext),
                    YtttButtonVariant::Primary,
                    theme,
                    cx,
                )
                .debug_selector(|| "onboarding-language-next".to_string())
                .on_click(cx.listener(|this, _, _window, cx| {
                    this.advance_onboarding();
                    cx.notify();
                })),
            ),
        )
}

fn layout_step(
    cx: &mut Context<WorkbenchView>,
    state: &OnboardingState,
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
            div()
                .flex()
                .justify_between()
                .child(
                    yttt_button(
                        "onboarding-language-back",
                        ui_text.get(UiTextKey::OnboardingBack),
                        YtttButtonVariant::Secondary,
                        theme,
                        cx,
                    )
                    .debug_selector(|| "onboarding-language-back".to_string())
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.return_to_onboarding_language();
                        cx.notify();
                    })),
                )
                .child(
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
    state: &OnboardingState,
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
                .h(rems(5.5))
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
                        "onboarding-agent-next",
                        ui_text.get(UiTextKey::OnboardingNext),
                        YtttButtonVariant::Primary,
                        theme,
                        cx,
                    )
                    .debug_selector(|| "onboarding-agent-next".to_string())
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.advance_onboarding();
                        cx.notify();
                    })),
                ),
        )
}

fn zed_import_step(
    cx: &mut Context<WorkbenchView>,
    state: &OnboardingState,
    ui_text: &UiText,
    theme: WorkbenchTheme,
) -> Div {
    let detection = &state.zed_detection;
    let has_themes = !detection.is_empty();
    let mut actions = div().flex().gap_2();
    if has_themes {
        actions = actions
            .child(
                yttt_button(
                    "onboarding-zed-skip",
                    ui_text.get(UiTextKey::OnboardingZedSkip),
                    YtttButtonVariant::Secondary,
                    theme,
                    cx,
                )
                .debug_selector(|| "onboarding-zed-skip".to_string())
                .on_click(cx.listener(|this, _, _window, cx| {
                    if let Err(error) = this.complete_onboarding(false) {
                        this.load_error = Some(error);
                    }
                    cx.notify();
                })),
            )
            .child(
                yttt_button(
                    "onboarding-zed-import",
                    ui_text.get(UiTextKey::OnboardingZedImport),
                    YtttButtonVariant::Primary,
                    theme,
                    cx,
                )
                .debug_selector(|| "onboarding-zed-import".to_string())
                .on_click(cx.listener(|this, _, _window, cx| {
                    if let Err(error) = this.complete_onboarding(true) {
                        this.load_error = Some(error);
                    }
                    cx.notify();
                })),
            );
    } else {
        actions = actions.child(
            yttt_button(
                "onboarding-zed-continue",
                ui_text.get(UiTextKey::OnboardingContinue),
                YtttButtonVariant::Primary,
                theme,
                cx,
            )
            .debug_selector(|| "onboarding-zed-continue".to_string())
            .on_click(cx.listener(|this, _, _window, cx| {
                if let Err(error) = this.complete_onboarding(false) {
                    this.load_error = Some(error);
                }
                cx.notify();
            })),
        );
    }

    div()
        .debug_selector(|| "onboarding-zed-import-step".to_string())
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
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child(ui_text.get(UiTextKey::OnboardingZedHeading)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_muted)
                        .child(ui_text.get(UiTextKey::OnboardingZedSubtitle)),
                ),
        )
        .when(!has_themes, |this| {
            this.child(
                div()
                    .rounded_lg()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.surface_elevated)
                    .p_4()
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child(ui_text.get(UiTextKey::OnboardingZedNoThemes)),
            )
        })
        .when(detection.ui_theme_count() > 0, |this| {
            this.child(detected_theme_panel(
                ui_text.get(UiTextKey::OnboardingZedUiThemes),
                &detection.extensions,
                false,
                theme,
            ))
        })
        .when(detection.icon_theme_count() > 0, |this| {
            this.child(detected_theme_panel(
                ui_text.get(UiTextKey::OnboardingZedIconThemes),
                &detection.extensions,
                true,
                theme,
            ))
        })
        .when(!detection.warnings.is_empty(), |this| {
            this.child(
                div()
                    .rounded_md()
                    .border_1()
                    .border_color(theme.warning)
                    .bg(theme.surface_elevated)
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(theme.warning)
                    .child(format!(
                        "{} ({})",
                        ui_text.get(UiTextKey::OnboardingZedDetectionWarnings),
                        detection.warnings.len()
                    )),
            )
        })
        .child(
            div()
                .flex()
                .justify_between()
                .child(
                    yttt_button(
                        "onboarding-zed-back",
                        ui_text.get(UiTextKey::OnboardingBack),
                        YtttButtonVariant::Secondary,
                        theme,
                        cx,
                    )
                    .debug_selector(|| "onboarding-zed-back".to_string())
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.return_to_onboarding_agent();
                        cx.notify();
                    })),
                )
                .child(actions),
        )
}

fn detected_theme_panel(
    title: &'static str,
    extensions: &[DetectedZedExtension],
    icon_themes: bool,
    theme: WorkbenchTheme,
) -> Div {
    let mut rows = div().flex().flex_col().gap_1();
    let id_prefix = if icon_themes {
        "onboarding-zed-icon-theme"
    } else {
        "onboarding-zed-ui-theme"
    };
    let mut index = 0usize;
    for extension in extensions {
        let names = if icon_themes {
            &extension.icon_theme_names
        } else {
            &extension.ui_theme_names
        };
        for name in names {
            let row_index = index;
            index += 1;
            rows = rows.child(
                div()
                    .id((id_prefix, row_index))
                    .debug_selector(move || format!("{id_prefix}-{row_index}"))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .rounded_md()
                    .px_3()
                    .py_2()
                    .bg(theme.surface_elevated)
                    .child(
                        div()
                            .min_w_0()
                            .text_sm()
                            .text_color(theme.text)
                            .truncate()
                            .child(name.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(theme.text_subtle)
                            .child(extension.name.clone()),
                    ),
            );
        }
    }

    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text_muted)
                .child(format!("{title} ({index})")),
        )
        .child(div().max_h(rems(11.25)).overflow_y_scrollbar().child(rows))
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
            .h(rems(4.5))
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
            .h(rems(4.5))
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
                    .h(rems(1.625))
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
