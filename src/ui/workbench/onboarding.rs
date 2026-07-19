use super::*;

const MONOSPACE_NERD_FONT_RECOMMENDATION_URL: &str = "https://font.subf.dev/zh-cn/";
const MONOSPACE_FONT_WIDTH_TOLERANCE: f32 = 0.01;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum OnboardingStep {
    #[default]
    Language,
    Font,
    Layout,
    Agent,
    ZedImport,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum OnboardingFontDetection {
    #[default]
    Pending,
    Missing,
    Recommended(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OnboardingState {
    pub step: OnboardingStep,
    pub selected_language: LanguageSetting,
    pub selected_layout: DefaultLayoutKind,
    pub selected_agent: BuiltinAgent,
    pub zed_detection: ZedThemeDetection,
    pub font_detection: OnboardingFontDetection,
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
            font_detection: OnboardingFontDetection::Pending,
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
    ui_style: UiStyle,
    terminal_font_select: Option<&Entity<SettingsFontFamilySelectState>>,
    command_palette_keybinding: Option<String>,
) -> Div {
    let step = match state.step {
        OnboardingStep::Language => language_step(cx, state, ui_text, theme, ui_style),
        OnboardingStep::Font => terminal_font_step(
            cx,
            ui_text,
            theme,
            ui_style,
            &state.font_detection,
            terminal_font_select.expect("terminal font select must exist on the font step"),
        ),
        OnboardingStep::Layout => layout_step(cx, state, ui_text, theme, ui_style),
        OnboardingStep::Agent => agent_step(cx, state, ui_text, theme, ui_style),
        OnboardingStep::ZedImport => zed_import_step(cx, state, ui_text, theme, ui_style),
    };

    div()
        .flex()
        .flex_1()
        .w_full()
        .min_h_0()
        .items_center()
        .justify_center()
        .px(ui_style.spacing.xxxl)
        .py(ui_style.spacing.xxl)
        .child(
            div()
                .flex()
                .w_full()
                .max_w(px(720.0))
                .flex_col()
                .gap(ui_style.spacing.xl + ui_style.spacing.xs)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(ui_style.spacing.md)
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
                    ui_style,
                    command_palette_keybinding,
                )),
        )
}

fn language_step(
    cx: &mut Context<WorkbenchView>,
    state: &OnboardingState,
    ui_text: &UiText,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    let mut language_picker = div().flex().w_full().gap(ui_style.spacing.lg);
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
                .rounded(ui_style.radius.card)
                .border(if selected {
                    ui_style.border.emphasized
                } else {
                    ui_style.border.hairline
                })
                .border_color(if selected { theme.accent } else { theme.border })
                .bg(if selected {
                    ui_style.active_background(theme)
                } else {
                    theme.surface_elevated
                })
                .hover(move |this| this.bg(ui_style.hover_background(theme)))
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
        .gap(ui_style.spacing.xl)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(ui_style.spacing.xs)
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
                    ui_style,
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

pub(super) fn font_family_has_fixed_ascii_width(window: &mut Window, font_family: &str) -> bool {
    let font = gpui::Font {
        family: font_family.to_string().into(),
        features: gpui::FontFeatures::disable_ligatures(),
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: gpui::FontStyle::Normal,
    };
    let mut expected_width: Option<Pixels> = None;

    for text in ["i", "W", "0", " "] {
        let run = gpui::TextRun {
            len: text.len(),
            font: font.clone(),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let width = window
            .text_system()
            .shape_line(text.into(), px(16.0), &[run], None)
            .width;
        if width <= px(0.0) {
            return false;
        }

        if let Some(expected_width) = expected_width {
            if (width.as_f32() - expected_width.as_f32()).abs() > MONOSPACE_FONT_WIDTH_TOLERANCE {
                return false;
            }
        } else {
            expected_width = Some(width);
        }
    }

    true
}

fn terminal_font_step(
    cx: &mut Context<WorkbenchView>,
    ui_text: &UiText,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    font_detection: &OnboardingFontDetection,
    font_select: &Entity<SettingsFontFamilySelectState>,
) -> Div {
    let select_style = yttt_select_style(theme, ui_style);
    let font_select = Select::new(font_select)
        .small()
        .menu_width(select_style.menu_width)
        .search_placeholder(ui_text.get(UiTextKey::SettingsSearchFont))
        .appearance(true)
        .cleanable(false)
        .w(select_style.width)
        .h(select_style.height)
        .rounded(select_style.radius)
        .bg(select_style.background)
        .border_color(select_style.border)
        .text_color(select_style.text);
    let recommendation = match font_detection {
        OnboardingFontDetection::Recommended(font_family) => div()
            .debug_selector(|| "onboarding-terminal-font-recommendation".to_string())
            .flex()
            .flex_col()
            .gap(ui_style.spacing.xs)
            .rounded(ui_style.radius.control)
            .border(ui_style.border.hairline)
            .border_color(theme.success)
            .bg(theme.surface_elevated)
            .px(ui_style.spacing.lg)
            .py(ui_style.spacing.md)
            .text_xs()
            .text_color(theme.success)
            .child(ui_text.get(UiTextKey::OnboardingFontDetectedRecommendation))
            .child(
                div()
                    .debug_selector(|| "onboarding-terminal-font-recommendation-name".to_string())
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(font_family.clone()),
            ),
        OnboardingFontDetection::Pending | OnboardingFontDetection::Missing => div()
            .debug_selector(|| "onboarding-terminal-font-recommendation".to_string())
            .flex()
            .flex_col()
            .gap(ui_style.spacing.xs)
            .rounded(ui_style.radius.control)
            .border(ui_style.border.hairline)
            .border_color(theme.warning)
            .bg(theme.surface_elevated)
            .px(ui_style.spacing.lg)
            .py(ui_style.spacing.md)
            .text_xs()
            .text_color(theme.warning)
            .child(ui_text.get(UiTextKey::OnboardingFontRecommendation))
            .child(
                div()
                    .id("onboarding-terminal-font-recommendation-link")
                    .debug_selector(|| "onboarding-terminal-font-recommendation-link".to_string())
                    .cursor_pointer()
                    .text_color(theme.accent)
                    .on_click(cx.listener(|_, _, _window, cx| {
                        cx.open_url(MONOSPACE_NERD_FONT_RECOMMENDATION_URL);
                    }))
                    .child(MONOSPACE_NERD_FONT_RECOMMENDATION_URL),
            ),
    };

    div()
        .debug_selector(|| "onboarding-terminal-font-step".to_string())
        .flex()
        .flex_col()
        .gap(ui_style.spacing.xl)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(ui_style.spacing.xs)
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child(ui_text.get(UiTextKey::OnboardingFontHeading)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_muted)
                        .child(ui_text.get(UiTextKey::OnboardingFontSubtitle)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(ui_style.spacing.lg)
                .rounded(ui_style.radius.card)
                .border(ui_style.border.hairline)
                .border_color(theme.border)
                .bg(theme.surface_elevated)
                .p(ui_style.spacing.xl)
                .child(
                    div()
                        .debug_selector(|| "onboarding-terminal-font-select".to_string())
                        .w(select_style.width)
                        .h(select_style.height)
                        .child(font_select),
                )
                .child(recommendation),
        )
        .child(
            div()
                .flex()
                .justify_between()
                .child(
                    yttt_button(
                        "onboarding-font-back",
                        ui_text.get(UiTextKey::OnboardingBack),
                        YtttButtonVariant::Secondary,
                        theme,
                        ui_style,
                        cx,
                    )
                    .debug_selector(|| "onboarding-font-back".to_string())
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.return_to_onboarding_language();
                        cx.notify();
                    })),
                )
                .child(
                    yttt_button(
                        "onboarding-font-next",
                        ui_text.get(UiTextKey::OnboardingNext),
                        YtttButtonVariant::Primary,
                        theme,
                        ui_style,
                        cx,
                    )
                    .debug_selector(|| "onboarding-font-next".to_string())
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
    ui_style: UiStyle,
) -> Div {
    let mut layout_picker = div().flex().w_full().gap(ui_style.spacing.lg);
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
                .gap(ui_style.spacing.lg)
                .rounded(ui_style.radius.card)
                .border(if selected {
                    ui_style.border.emphasized
                } else {
                    ui_style.border.hairline
                })
                .border_color(if selected { theme.accent } else { theme.border })
                .bg(if selected {
                    ui_style.active_background(theme)
                } else {
                    theme.surface_elevated
                })
                .p(ui_style.spacing.xl)
                .hover(move |this| this.bg(ui_style.hover_background(theme)))
                .on_click(cx.listener(move |this, _, _window, cx| {
                    this.select_onboarding_layout(layout_kind);
                    cx.notify();
                }))
                .child(layout_preview(
                    layout_kind,
                    state.selected_agent,
                    ui_text,
                    theme,
                    ui_style,
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(ui_style.spacing.xs)
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
        .gap(ui_style.spacing.lg)
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
                        "onboarding-layout-back",
                        ui_text.get(UiTextKey::OnboardingBack),
                        YtttButtonVariant::Secondary,
                        theme,
                        ui_style,
                        cx,
                    )
                    .debug_selector(|| "onboarding-layout-back".to_string())
                    .on_click(cx.listener(|this, _, _window, cx| {
                        this.return_to_onboarding_terminal_font();
                        cx.notify();
                    })),
                )
                .child(
                    yttt_button(
                        "onboarding-next",
                        ui_text.get(UiTextKey::OnboardingNext),
                        YtttButtonVariant::Primary,
                        theme,
                        ui_style,
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
    ui_style: UiStyle,
) -> Div {
    let mut agent_picker = div().flex().w_full().gap(ui_style.spacing.md);
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
                .gap(ui_style.spacing.md)
                .h(rems(5.5))
                .rounded(ui_style.radius.card)
                .border(if selected {
                    ui_style.border.emphasized
                } else {
                    ui_style.border.hairline
                })
                .border_color(if selected { theme.accent } else { theme.border })
                .bg(if selected {
                    ui_style.active_background(theme)
                } else {
                    theme.surface_elevated
                })
                .hover(move |this| this.bg(ui_style.hover_background(theme)))
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
        .gap(ui_style.spacing.xl)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(ui_style.spacing.xs)
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
                .gap(ui_style.spacing.md)
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
                    ui_style,
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
                        ui_style,
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
                        ui_style,
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
    ui_style: UiStyle,
) -> Div {
    let detection = &state.zed_detection;
    let has_themes = !detection.is_empty();
    let mut actions = div().flex().gap(ui_style.spacing.md);
    if has_themes {
        actions = actions
            .child(
                yttt_button(
                    "onboarding-zed-skip",
                    ui_text.get(UiTextKey::OnboardingZedSkip),
                    YtttButtonVariant::Secondary,
                    theme,
                    ui_style,
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
                    ui_style,
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
                ui_style,
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
        .gap(ui_style.spacing.xl)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(ui_style.spacing.xs)
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
                    .rounded(ui_style.radius.card)
                    .border(ui_style.border.hairline)
                    .border_color(theme.border)
                    .bg(theme.surface_elevated)
                    .p(ui_style.spacing.xl)
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
                ui_style,
            ))
        })
        .when(detection.icon_theme_count() > 0, |this| {
            this.child(detected_theme_panel(
                ui_text.get(UiTextKey::OnboardingZedIconThemes),
                &detection.extensions,
                true,
                theme,
                ui_style,
            ))
        })
        .when(!detection.warnings.is_empty(), |this| {
            this.child(
                div()
                    .rounded(ui_style.radius.control)
                    .border(ui_style.border.hairline)
                    .border_color(theme.warning)
                    .bg(theme.surface_elevated)
                    .px(ui_style.spacing.lg)
                    .py(ui_style.spacing.md)
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
                        ui_style,
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
    ui_style: UiStyle,
) -> Div {
    let mut rows = div().flex().flex_col().gap(ui_style.spacing.xs);
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
                    .gap(ui_style.spacing.lg)
                    .rounded(ui_style.radius.control)
                    .px(ui_style.spacing.lg)
                    .py(ui_style.spacing.md)
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
        .gap(ui_style.spacing.md)
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
    ui_style: UiStyle,
) -> Div {
    match layout_kind {
        DefaultLayoutKind::SplitPane => div()
            .flex()
            .h(rems(4.5))
            .w_full()
            .overflow_hidden()
            .rounded(ui_style.radius.control)
            .border(ui_style.border.hairline)
            .border_color(theme.border)
            .bg(theme.surface_elevated)
            .child(
                div()
                    .flex()
                    .flex_basis(relative(0.65))
                    .items_center()
                    .justify_center()
                    .border_r(ui_style.border.hairline)
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
            .rounded(ui_style.radius.control)
            .border(ui_style.border.hairline)
            .border_color(theme.border)
            .bg(theme.surface_elevated)
            .child(
                div()
                    .flex()
                    .h(rems(1.625))
                    .border_b(ui_style.border.hairline)
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .border_r(ui_style.border.hairline)
                            .border_color(theme.border)
                            .bg(ui_style.active_background(theme))
                            .px(ui_style.spacing.lg)
                            .text_xs()
                            .text_color(theme.text)
                            .child(ui_text.get(UiTextKey::OnboardingAgentPane)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .px(ui_style.spacing.lg)
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
    ui_style: UiStyle,
    command_palette_keybinding: Option<String>,
) -> Div {
    let mut hint = div()
        .flex()
        .items_center()
        .justify_center()
        .gap(ui_style.spacing.md)
        .text_xs()
        .text_color(theme.text_muted)
        .child(ui_text.get(UiTextKey::OnboardingCommandPaletteHint));
    if let Some(keybinding) = command_palette_keybinding {
        hint = hint.child(workbench_keybinding_badge(keybinding, theme, ui_style));
    }
    hint.child(
        yttt_button(
            "onboarding-open-command-palette",
            ui_text.get(UiTextKey::CommandPalette),
            YtttButtonVariant::Ghost,
            theme,
            ui_style,
            cx,
        )
        .debug_selector(|| "onboarding-open-command-palette".to_string())
        .on_click(cx.listener(|this, _, window, cx| {
            this.on_open_command_palette(&OpenCommandPalette, window, cx);
        })),
    )
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use gpui::AppContext as _;
    use tempfile::tempdir;

    use super::*;

    #[gpui::test]
    fn detected_font_still_shows_the_recommended_family(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let temp = tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let root_slot = Rc::new(RefCell::new(None));
        let root_slot_for_window = root_slot.clone();
        let (_component_root, cx) = cx.add_window_view(move |window, cx| {
            let root = cx.new(|_| WorkbenchView::with_config_paths(paths));
            *root_slot_for_window.borrow_mut() = Some(root.clone());
            gpui_component::Root::new(root, window, cx)
        });
        let root = root_slot.borrow_mut().take().unwrap();

        root.update(cx, |root, cx| {
            let onboarding = root.onboarding.as_mut().expect("onboarding must be active");
            onboarding.step = OnboardingStep::Font;
            onboarding.font_detection =
                OnboardingFontDetection::Recommended("Maple Mono NF CN".to_string());
            cx.notify();
        });
        cx.run_until_parked();

        assert!(
            cx.debug_bounds("onboarding-terminal-font-recommendation")
                .is_some()
        );
        assert!(
            cx.debug_bounds("onboarding-terminal-font-recommendation-name")
                .is_some()
        );
        assert!(
            cx.debug_bounds("onboarding-terminal-font-recommendation-link")
                .is_none()
        );
        cx.read(|app| {
            assert_eq!(
                root.read(app)
                    .onboarding
                    .as_ref()
                    .map(|state| &state.font_detection),
                Some(&OnboardingFontDetection::Recommended(
                    "Maple Mono NF CN".to_string()
                ))
            );
        });
    }
}
