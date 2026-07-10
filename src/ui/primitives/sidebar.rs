use gpui::{Pixels, Rgba, px};

use crate::ui::theme::WorkbenchTheme;

pub const PROJECT_SIDEBAR_MIN_WIDTH: f32 = 160.0;
pub const PROJECT_SIDEBAR_MAX_WIDTH: f32 = 420.0;
pub const PROJECT_SIDEBAR_DEFAULT_WIDTH: f32 = 216.0;
pub const PROJECT_SIDEBAR_COLLAPSED_WIDTH: f32 = 46.0;
pub const PROJECT_FILE_PANEL_MIN_WIDTH: f32 = 200.0;
pub const PROJECT_FILE_PANEL_MAX_WIDTH: f32 = 520.0;
pub const SIDEBAR_RESIZE_HIT_AREA_WIDTH: f32 = 5.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarSide {
    Left,
    Right,
}

pub fn resize_sidebar_width(
    side: SidebarSide,
    current_width: f32,
    pointer_delta_x: f32,
    min_width: f32,
    max_width: f32,
) -> f32 {
    let current_width = if current_width.is_finite() {
        current_width
    } else {
        min_width
    };
    let pointer_delta_x = if pointer_delta_x.is_finite() {
        pointer_delta_x
    } else {
        0.0
    };
    let signed_delta = match side {
        SidebarSide::Left => pointer_delta_x,
        SidebarSide::Right => -pointer_delta_x,
    };
    (current_width + signed_delta).clamp(min_width, max_width)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SidebarWidthState {
    side: SidebarSide,
    expanded_width: f32,
    min_width: f32,
    max_width: f32,
    inactive_width: f32,
    active: bool,
}

impl SidebarWidthState {
    pub fn new(
        side: SidebarSide,
        expanded_width: f32,
        min_width: f32,
        max_width: f32,
        inactive_width: f32,
    ) -> Self {
        Self {
            side,
            expanded_width: resize_sidebar_width(
                SidebarSide::Left,
                expanded_width,
                0.0,
                min_width,
                max_width,
            ),
            min_width,
            max_width,
            inactive_width,
            active: true,
        }
    }

    pub fn side(&self) -> SidebarSide {
        self.side
    }

    pub fn expanded_width(&self) -> f32 {
        self.expanded_width
    }

    pub fn visible_width(&self) -> f32 {
        if self.active {
            self.expanded_width
        } else {
            self.inactive_width
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn set_active(&mut self, active: bool) {
        self.active = active;
    }

    pub fn set_expanded_width(&mut self, width: f32) {
        self.expanded_width = resize_sidebar_width(
            SidebarSide::Left,
            width,
            0.0,
            self.min_width,
            self.max_width,
        );
    }

    pub fn apply_pointer_delta(&mut self, pointer_delta_x: f32) -> f32 {
        self.expanded_width = resize_sidebar_width(
            self.side,
            self.expanded_width,
            pointer_delta_x,
            self.min_width,
            self.max_width,
        );
        self.expanded_width
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttSidebarStyle {
    pub width: Pixels,
    pub default_width: Pixels,
    pub min_width: Pixels,
    pub max_width: Pixels,
    pub collapsed_width: Pixels,
    pub border_width: Pixels,
    pub resize_hit_area_width: Pixels,
    pub item_height: Pixels,
    pub item_padding_x: Pixels,
    pub background: Rgba,
    pub active_background: Rgba,
    pub hover_background: Rgba,
}

pub fn yttt_sidebar_style(theme: WorkbenchTheme) -> YtttSidebarStyle {
    YtttSidebarStyle {
        width: px(PROJECT_SIDEBAR_DEFAULT_WIDTH),
        default_width: px(PROJECT_SIDEBAR_DEFAULT_WIDTH),
        min_width: px(PROJECT_SIDEBAR_MIN_WIDTH),
        max_width: px(PROJECT_SIDEBAR_MAX_WIDTH),
        collapsed_width: px(PROJECT_SIDEBAR_COLLAPSED_WIDTH),
        border_width: px(1.0),
        resize_hit_area_width: px(SIDEBAR_RESIZE_HIT_AREA_WIDTH),
        item_height: px(28.0),
        item_padding_x: px(8.0),
        background: theme.app_background,
        active_background: theme.active_surface,
        hover_background: theme.hover_surface,
    }
}
