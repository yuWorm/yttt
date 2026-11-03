mod item;
mod state;
mod view;

pub use item::PickerItem;
pub use state::{PickerState, query_match};
pub use view::{
    PickerFooterAction, PickerOverlayRow, PickerText, picker_header, picker_overlay,
    picker_overlay_with_preview, picker_panel, picker_row, picker_scroll_anchor_index,
    visible_row_range,
};

pub trait PickerDelegate {
    fn items(&self) -> &[PickerItem];
}
