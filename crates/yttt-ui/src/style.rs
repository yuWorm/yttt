use gpui::{Pixels, Rems, Rgba, px, rems, rgba};

use crate::theme::WorkbenchTheme;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UiStyleId {
    #[default]
    Zed,
    Rounded,
}

impl UiStyleId {
    pub const ALL: [Self; 2] = [Self::Zed, Self::Rounded];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Zed => "zed",
            Self::Rounded => "rounded",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Zed => "Zed",
            Self::Rounded => "Rounded",
        }
    }

    pub fn from_display_name(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|style| style.display_name() == value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiComponentThemeStyle {
    pub radius: usize,
    pub radius_lg: usize,
    pub shadow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiSpacingScale {
    pub xxs: Rems,
    pub xs: Rems,
    pub sm: Rems,
    pub md: Rems,
    pub lg: Rems,
    pub xl: Rems,
    pub xxl: Rems,
    pub xxxl: Rems,
    pub overlay_top: Rems,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiRadiusScale {
    pub compact: Pixels,
    pub action: Pixels,
    pub control: Pixels,
    pub input: Pixels,
    pub card: Pixels,
    pub surface: Pixels,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiBorderScale {
    pub hairline: Pixels,
    pub emphasized: Pixels,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiControlMetrics {
    pub button_height: Rems,
    pub button_padding_x: Rems,
    pub settings_height: Rems,
    pub toolbar_height: Rems,
    pub dialog_input_height: Rems,
    pub palette_input_height: Rems,
    pub search_height: Rems,
    pub palette_footer_height: Rems,
    pub status_footer_height: Rems,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiRowMetrics {
    pub palette_height: Rems,
    pub settings_height: Rems,
    pub sidebar_height: Rems,
    pub tab_height: Rems,
    pub diff_sidebar_height: Rems,
    pub palette_padding_x: Rems,
    pub settings_padding_y: Rems,
    pub sidebar_padding_x: Rems,
    pub tab_padding_x: Rems,
    pub palette_radius: Pixels,
    pub settings_radius: Pixels,
    pub sidebar_radius: Pixels,
    pub tab_radius: Pixels,
    pub diff_sidebar_radius: Pixels,
    pub palette_border_width: Pixels,
    pub settings_border_width: Pixels,
    pub sidebar_border_width: Pixels,
    pub tab_border_width: Pixels,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiIconButtonMetrics {
    pub toolbar_size: Rems,
    pub sidebar_header_size: Rems,
    pub tab_close_size: Rems,
    pub overlay_close_size: Rems,
    pub icon_size: Rems,
    pub toolbar_radius: Pixels,
    pub sidebar_header_radius: Pixels,
    pub tab_close_radius: Pixels,
    pub overlay_close_radius: Pixels,
    pub toolbar_border_width: Pixels,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiPanelVisualMetrics {
    pub radius: Pixels,
    pub dialog_padding: Pixels,
    pub panel_overlay: Rgba,
    pub dialog_overlay: Rgba,
    pub fullscreen_overlay: Rgba,
    pub editor_overlay: Rgba,
    pub shadow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiNotificationMetrics {
    pub min_height: Rems,
    pub padding_x: Rems,
    pub padding_y: Rems,
    pub gap: Rems,
    pub radius: Pixels,
    pub border_width: Pixels,
    pub icon_size: Rems,
    pub action_padding_x: Rems,
    pub action_padding_y: Rems,
    pub action_radius: Pixels,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiSwitchMetrics {
    pub width: Rems,
    pub height: Rems,
    pub track_width: Rems,
    pub track_height: Rems,
    pub track_padding: Rems,
    pub thumb_size: Rems,
    pub control_height: Rems,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiStyle {
    pub id: UiStyleId,
    pub component: UiComponentThemeStyle,
    pub spacing: UiSpacingScale,
    pub radius: UiRadiusScale,
    pub border: UiBorderScale,
    pub controls: UiControlMetrics,
    pub rows: UiRowMetrics,
    pub icon_buttons: UiIconButtonMetrics,
    pub panels: UiPanelVisualMetrics,
    pub notifications: UiNotificationMetrics,
    pub switches: UiSwitchMetrics,
    pub status_dot_size: Pixels,
    pub hover_accent_alpha: f32,
    pub active_accent_alpha: f32,
}

impl UiStyle {
    pub fn resolve(id: UiStyleId) -> Self {
        match id {
            UiStyleId::Zed => Self::zed(),
            UiStyleId::Rounded => Self::rounded(),
        }
    }

    pub fn hover_background(self, theme: WorkbenchTheme) -> Rgba {
        if self.hover_accent_alpha == 0.0 {
            theme.hover_surface
        } else {
            theme.surface.blend(Rgba {
                a: self.hover_accent_alpha,
                ..theme.accent
            })
        }
    }

    pub fn active_background(self, theme: WorkbenchTheme) -> Rgba {
        if self.active_accent_alpha == 0.0 {
            theme.active_surface
        } else {
            theme.surface.blend(Rgba {
                a: self.active_accent_alpha,
                ..theme.accent
            })
        }
    }

    fn zed() -> Self {
        let border = UiBorderScale {
            hairline: px(1.0),
            emphasized: px(2.0),
        };
        let radius = UiRadiusScale {
            compact: px(4.0),
            action: px(5.0),
            control: px(6.0),
            input: px(7.0),
            card: px(8.0),
            surface: px(8.0),
        };

        Self {
            id: UiStyleId::Zed,
            component: UiComponentThemeStyle {
                radius: 6,
                radius_lg: 8,
                shadow: false,
            },
            spacing: UiSpacingScale {
                xxs: rems(0.125),
                xs: rems(0.25),
                sm: rems(0.375),
                md: rems(0.5),
                lg: rems(0.75),
                xl: rems(1.0),
                xxl: rems(1.5),
                xxxl: rems(2.0),
                overlay_top: rems(4.0),
            },
            radius,
            border,
            controls: UiControlMetrics {
                button_height: rems(1.25),
                button_padding_x: rems(0.25),
                settings_height: rems(2.0),
                toolbar_height: rems(1.875),
                dialog_input_height: rems(2.125),
                palette_input_height: rems(2.625),
                search_height: rems(2.25),
                palette_footer_height: rems(2.75),
                status_footer_height: rems(2.875),
            },
            rows: UiRowMetrics {
                palette_height: rems(3.375),
                settings_height: rems(4.5),
                sidebar_height: rems(1.75),
                tab_height: rems(2.0),
                diff_sidebar_height: rems(2.375),
                palette_padding_x: rems(0.75),
                settings_padding_y: rems(0.75),
                sidebar_padding_x: rems(0.5),
                tab_padding_x: rems(0.5),
                palette_radius: radius.control,
                settings_radius: px(0.0),
                sidebar_radius: radius.control,
                tab_radius: px(0.0),
                diff_sidebar_radius: px(0.0),
                palette_border_width: border.hairline,
                settings_border_width: border.hairline,
                sidebar_border_width: px(0.0),
                tab_border_width: border.hairline,
            },
            icon_buttons: UiIconButtonMetrics {
                toolbar_size: rems(1.75),
                sidebar_header_size: rems(1.5),
                tab_close_size: rems(1.0),
                overlay_close_size: rems(1.75),
                icon_size: rems(0.75),
                toolbar_radius: px(0.0),
                sidebar_header_radius: radius.compact,
                tab_close_radius: radius.compact,
                overlay_close_radius: radius.control,
                toolbar_border_width: border.hairline,
            },
            panels: UiPanelVisualMetrics {
                radius: radius.surface,
                dialog_padding: px(16.0),
                panel_overlay: rgba(0x00000066),
                dialog_overlay: rgba(0x00000073),
                fullscreen_overlay: rgba(0x000000b3),
                editor_overlay: rgba(0x00000099),
                shadow: false,
            },
            notifications: UiNotificationMetrics {
                min_height: rems(2.75),
                padding_x: rems(0.75),
                padding_y: rems(0.5),
                gap: rems(0.5),
                radius: radius.surface,
                border_width: border.hairline,
                icon_size: rems(0.875),
                action_radius: radius.action,
                action_padding_x: rems(0.375),
                action_padding_y: rems(0.125),
            },
            switches: UiSwitchMetrics {
                width: rems(2.625),
                height: rems(1.625),
                track_width: rems(2.125),
                track_height: rems(1.25),
                track_padding: rems(0.125),
                thumb_size: rems(0.875),
                control_height: rems(2.0),
            },
            status_dot_size: px(6.0),
            hover_accent_alpha: 0.0,
            active_accent_alpha: 0.0,
        }
    }

    fn rounded() -> Self {
        let border = UiBorderScale {
            hairline: px(1.0),
            emphasized: px(2.0),
        };
        let radius = UiRadiusScale {
            compact: px(7.0),
            action: px(8.0),
            control: px(10.0),
            input: px(12.0),
            card: px(12.0),
            surface: px(16.0),
        };

        Self {
            id: UiStyleId::Rounded,
            component: UiComponentThemeStyle {
                radius: 10,
                radius_lg: 16,
                shadow: true,
            },
            spacing: UiSpacingScale {
                xxs: rems(0.125),
                xs: rems(0.375),
                sm: rems(0.5),
                md: rems(0.625),
                lg: rems(0.875),
                xl: rems(1.25),
                xxl: rems(1.75),
                xxxl: rems(2.25),
                overlay_top: rems(4.0),
            },
            radius,
            border,
            controls: UiControlMetrics {
                button_height: rems(1.75),
                button_padding_x: rems(0.75),
                settings_height: rems(2.25),
                toolbar_height: rems(2.25),
                dialog_input_height: rems(2.5),
                palette_input_height: rems(2.875),
                search_height: rems(2.5),
                palette_footer_height: rems(3.25),
                status_footer_height: rems(3.25),
            },
            rows: UiRowMetrics {
                palette_height: rems(3.75),
                settings_height: rems(5.0),
                sidebar_height: rems(2.25),
                tab_height: rems(2.5),
                diff_sidebar_height: rems(2.75),
                palette_padding_x: rems(0.875),
                settings_padding_y: rems(0.875),
                sidebar_padding_x: rems(0.75),
                tab_padding_x: rems(0.75),
                palette_radius: radius.card,
                settings_radius: radius.card,
                sidebar_radius: radius.control,
                tab_radius: radius.control,
                diff_sidebar_radius: radius.control,
                palette_border_width: border.hairline,
                settings_border_width: border.hairline,
                sidebar_border_width: border.hairline,
                tab_border_width: border.hairline,
            },
            icon_buttons: UiIconButtonMetrics {
                toolbar_size: rems(1.875),
                sidebar_header_size: rems(1.75),
                tab_close_size: rems(1.25),
                overlay_close_size: rems(2.0),
                icon_size: rems(0.875),
                toolbar_radius: radius.compact,
                sidebar_header_radius: radius.compact,
                tab_close_radius: radius.compact,
                overlay_close_radius: radius.control,
                toolbar_border_width: border.hairline,
            },
            panels: UiPanelVisualMetrics {
                radius: radius.surface,
                dialog_padding: px(20.0),
                panel_overlay: rgba(0x00000073),
                dialog_overlay: rgba(0x00000080),
                shadow: true,
                fullscreen_overlay: rgba(0x000000c0),
                editor_overlay: rgba(0x000000b3),
            },
            notifications: UiNotificationMetrics {
                min_height: rems(3.25),
                padding_x: rems(0.875),
                padding_y: rems(0.75),
                gap: rems(0.625),
                radius: px(14.0),
                border_width: border.hairline,
                icon_size: rems(1.0),
                action_radius: radius.action,
                action_padding_x: rems(0.5),
                action_padding_y: rems(0.25),
            },
            switches: UiSwitchMetrics {
                width: rems(2.75),
                height: rems(1.75),
                track_width: rems(2.25),
                track_height: rems(1.375),
                track_padding: rems(0.125),
                thumb_size: rems(1.0),
                control_height: rems(2.25),
            },
            status_dot_size: px(8.0),
            hover_accent_alpha: 0.1,
            active_accent_alpha: 0.16,
        }
    }
}

impl Default for UiStyle {
    fn default() -> Self {
        Self::resolve(UiStyleId::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::WorkbenchTheme;

    #[test]
    fn default_profile_preserves_zed_metrics() {
        let style = UiStyle::default();

        assert_eq!(style.id, UiStyleId::Zed);
        assert_eq!(style.radius.surface, px(8.0));
        assert!(!style.panels.shadow);
        assert_eq!(
            style.hover_background(WorkbenchTheme::one_dark()),
            WorkbenchTheme::one_dark().hover_surface
        );
    }

    #[test]
    fn rounded_profile_changes_global_geometry_and_interaction_tokens() {
        let zed = UiStyle::resolve(UiStyleId::Zed);
        let rounded = UiStyle::resolve(UiStyleId::Rounded);
        let theme = WorkbenchTheme::one_dark();

        assert!(rounded.radius.surface > zed.radius.surface);
        assert!(rounded.spacing.md.0 > zed.spacing.md.0);
        assert!(rounded.controls.toolbar_height.0 > zed.controls.toolbar_height.0);
        assert!(rounded.rows.sidebar_height.0 > zed.rows.sidebar_height.0);
        assert!(rounded.panels.shadow);
        assert_ne!(rounded.hover_background(theme), zed.hover_background(theme));
        assert_ne!(
            rounded.active_background(theme),
            zed.active_background(theme)
        );
    }
}
