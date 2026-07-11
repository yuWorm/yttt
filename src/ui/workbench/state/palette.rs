use gpui::{Entity, ScrollHandle, Subscription};
use gpui_component::input::InputState;

use crate::palette::{ActivePalette, RecentProject};

pub(in super::super) struct PaletteControllerState {
    pub(in super::super) active_palette: Option<ActivePalette>,
    pub(in super::super) recent_projects: Vec<RecentProject>,
    pub(in super::super) input: Option<Entity<InputState>>,
    pub(in super::super) input_subscription: Option<Subscription>,
    pub(in super::super) input_needs_focus: bool,
    pub(in super::super) scroll_handle: ScrollHandle,
}

impl PaletteControllerState {
    pub(in super::super) fn new(recent_projects: Vec<RecentProject>) -> Self {
        Self {
            active_palette: None,
            recent_projects,
            input: None,
            input_subscription: None,
            input_needs_focus: false,
            scroll_handle: ScrollHandle::new(),
        }
    }
}
