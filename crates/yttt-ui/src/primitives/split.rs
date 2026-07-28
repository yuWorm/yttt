use gpui::Pixels;

use crate::theme::WorkbenchTheme;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttSplitHandleStyle {
    pub visible_line_width: Pixels,
    pub hit_area_width: Pixels,
}

pub fn yttt_split_handle_style(theme: WorkbenchTheme) -> YtttSplitHandleStyle {
    YtttSplitHandleStyle {
        visible_line_width: theme.split_line_width,
        hit_area_width: theme.split_hit_area_width,
    }
}
