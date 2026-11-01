use gpui::{BoxShadow, Styled, hsla, px};
use gpui_component::box_shadow;

/// How far a surface floats above the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiElevation {
    /// Tooltips and menus.
    Floating,
    /// Dialogs, palettes, and notifications.
    Modal,
}

pub fn floating_shadows() -> Vec<BoxShadow> {
    vec![
        box_shadow(px(0.), px(2.), px(3.), px(0.), hsla(0., 0., 0., 0.12)),
        box_shadow(px(0.), px(1.), px(0.), px(0.), hsla(0., 0., 0., 0.06)),
    ]
}

pub trait YtttSurface: Styled + Sized {
    fn yttt_elevation(self, elevation: UiElevation) -> Self {
        match elevation {
            UiElevation::Floating => self.shadow(floating_shadows()),
            UiElevation::Modal => self.shadow_lg(),
        }
    }
}

impl<T: Styled + Sized> YtttSurface for T {}
