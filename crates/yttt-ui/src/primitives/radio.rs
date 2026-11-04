#![allow(clippy::disallowed_methods)]

use gpui::{ElementId, SharedString};
use gpui_component::{
    Sizable as _,
    radio::{Radio, RadioGroup},
};

pub fn yttt_radio(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
) -> Radio {
    let label: SharedString = label.into();
    Radio::new(id).small().label(label).checked(checked)
}

/// A row of mutually exclusive choices.
pub fn yttt_horizontal_radio_group(id: impl Into<ElementId>) -> RadioGroup {
    RadioGroup::horizontal(id)
}
