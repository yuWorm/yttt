use gpui::{Pixels, Rgba, px, rgb};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkbenchTheme {
    pub app_background: Rgba,
    pub surface: Rgba,
    pub surface_elevated: Rgba,
    pub panel_background: Rgba,
    pub editor_background: Rgba,
    pub tab_active_background: Rgba,
    pub tab_inactive_background: Rgba,
    pub titlebar_background: Rgba,
    pub titlebar_inactive_background: Rgba,
    pub toolbar_background: Rgba,
    pub statusbar_background: Rgba,
    pub sidebar_background: Rgba,
    pub tabbar_background: Rgba,
    pub terminal_background: Rgba,
    pub border: Rgba,
    pub border_strong: Rgba,
    pub border_variant: Rgba,
    pub border_focused: Rgba,
    pub split_line: Rgba,
    pub split_line_active: Rgba,
    pub text: Rgba,
    pub text_muted: Rgba,
    pub text_subtle: Rgba,
    pub text_disabled: Rgba,
    pub icon: Rgba,
    pub icon_muted: Rgba,
    pub icon_disabled: Rgba,
    pub icon_accent: Rgba,
    pub accent: Rgba,
    pub active_surface: Rgba,
    pub hover_surface: Rgba,
    pub element_background: Rgba,
    pub element_hover: Rgba,
    pub element_active: Rgba,
    pub element_selected: Rgba,
    pub element_disabled: Rgba,
    pub ghost_element_background: Rgba,
    pub ghost_element_hover: Rgba,
    pub ghost_element_active: Rgba,
    pub ghost_element_selected: Rgba,
    pub ghost_element_disabled: Rgba,
    pub danger: Rgba,
    pub success: Rgba,
    pub warning: Rgba,
    pub focus_ring: Rgba,
    pub selection: Rgba,
    pub focused_pane_border: Rgba,
    pub scrollbar_thumb: Rgba,
    pub scrollbar_thumb_hover: Rgba,
    pub split_line_width: Pixels,
    pub split_hit_area_width: Pixels,
}

impl WorkbenchTheme {
    pub fn one_dark() -> Self {
        Self {
            app_background: rgb(0x3b414d),
            surface: rgb(0x2f343e),
            surface_elevated: rgb(0x2f343e),
            panel_background: rgb(0x2f343e),
            editor_background: rgb(0x282c33),
            tab_active_background: rgb(0x282c33),
            tab_inactive_background: rgb(0x2f343e),
            titlebar_background: rgb(0x3b414d),
            titlebar_inactive_background: rgb(0x2e343e),
            toolbar_background: rgb(0x282c33),
            statusbar_background: rgb(0x3b414d),
            sidebar_background: rgb(0x2f343e),
            tabbar_background: rgb(0x2f343e),
            terminal_background: rgb(0x282c33),
            border: rgb(0x464b57),
            border_strong: rgb(0x464b57),
            border_variant: rgb(0x363c46),
            border_focused: rgb(0x47679e),
            split_line: rgb(0x363c46),
            split_line_active: rgb(0x47679e),
            text: rgb(0xdce0e5),
            text_muted: rgb(0xa9afbc),
            text_subtle: rgb(0x878a98),
            text_disabled: rgb(0x878a98),
            icon: rgb(0xdce0e5),
            icon_muted: rgb(0xa9afbc),
            icon_disabled: rgb(0x878a98),
            icon_accent: rgb(0x74ade8),
            accent: rgb(0x74ade8),
            active_surface: rgb(0x454a56),
            hover_surface: rgb(0x363c46),
            element_background: rgb(0x2e343e),
            element_hover: rgb(0x363c46),
            element_active: rgb(0x454a56),
            element_selected: rgb(0x454a56),
            element_disabled: rgb(0x2e343e),
            ghost_element_background: gpui::rgba(0x00000000),
            ghost_element_hover: rgb(0x363c46),
            ghost_element_active: rgb(0x454a56),
            ghost_element_selected: rgb(0x454a56),
            ghost_element_disabled: rgb(0x2e343e),
            danger: rgb(0xc24038),
            success: rgb(0xa5e075),
            warning: rgb(0xd19a66),
            focus_ring: rgb(0x47679e),
            selection: gpui::rgba(0x67769640),
            focused_pane_border: rgb(0x47679e),
            scrollbar_thumb: gpui::rgba(0xc8ccd44c),
            scrollbar_thumb_hover: rgb(0x363c46),
            split_line_width: px(1.0),
            split_hit_area_width: px(7.0),
        }
    }

    /// Base colors used when a Zed theme leaves a role unspecified.
    /// UI roles are independent of editor syntax and diagnostic colors.
    pub fn zed_defaults(dark: bool) -> Self {
        let mut theme = Self::one_dark();
        let (
            background,
            surface,
            element,
            hover,
            active,
            disabled,
            border,
            variant,
            focused,
            text,
            muted,
            placeholder,
            text_disabled,
            icon,
            accent,
        ) = if dark {
            (
                0x111110, 0x191918, 0x222221, 0xfefef31b, 0xfbfbeb23, 0xf6f6f513, 0x3b3a37,
                0x31312e, 0x004074, 0xeeeeec, 0xb5b3ad, 0x7c7b74, 0x6f6d66, 0xb5b3ad, 0x70b8ff,
            )
        } else {
            (
                0xfdfdfc, 0xf9f9f8, 0xf1f0ef, 0x1f150019, 0x1f180021, 0x20100010, 0xdad9d6,
                0xe2e1de, 0xc2e5ff, 0x21201c, 0x82827c, 0x82827c, 0x8d8d86, 0x63635e, 0x0d74ce,
            )
        };
        theme.app_background = rgb(background);
        theme.surface = rgb(surface);
        theme.surface_elevated = rgb(surface);
        theme.panel_background = rgb(surface);
        theme.sidebar_background = rgb(surface);
        theme.editor_background = rgb(background);
        theme.terminal_background = rgb(background);
        theme.titlebar_background = rgb(surface);
        theme.titlebar_inactive_background = rgb(element);
        theme.toolbar_background = rgb(background);
        theme.statusbar_background = rgb(surface);
        theme.tabbar_background = rgb(surface);
        theme.tab_inactive_background = rgb(surface);
        theme.tab_active_background = rgb(background);
        theme.border = rgb(border);
        theme.border_strong = rgb(border);
        theme.border_variant = rgb(variant);
        theme.border_focused = rgb(focused);
        theme.split_line = rgb(variant);
        theme.split_line_active = rgb(focused);
        theme.focus_ring = rgb(focused);
        theme.focused_pane_border = rgb(focused);
        theme.text = rgb(text);
        theme.text_muted = rgb(muted);
        theme.text_subtle = rgb(placeholder);
        theme.text_disabled = rgb(text_disabled);
        theme.icon = rgb(icon);
        theme.icon_muted = rgb(placeholder);
        theme.icon_disabled = rgb(text_disabled);
        theme.icon_accent = rgb(accent);
        theme.accent = rgb(accent);
        theme.element_background = rgb(element);
        theme.element_hover = gpui::rgba(hover);
        theme.element_active = gpui::rgba(active);
        theme.element_selected = gpui::rgba(active);
        theme.element_disabled = gpui::rgba(disabled);
        theme.ghost_element_hover = gpui::rgba(if dark { hover } else { disabled });
        theme.ghost_element_active = gpui::rgba(if dark { active } else { hover });
        theme.ghost_element_selected = gpui::rgba(active);
        theme.ghost_element_disabled = gpui::rgba(disabled);
        theme.hover_surface = theme.ghost_element_hover;
        theme.active_surface = theme.ghost_element_selected;
        theme
    }
}
