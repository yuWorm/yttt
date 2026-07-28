use gpui::{ElementId, SharedString};
use gpui_component::{Sizable as _, radio::Radio};

pub fn yttt_radio(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
) -> Radio {
    let label: SharedString = label.into();
    Radio::new(id).small().label(label).checked(checked)
}
