use gpui::{ElementId, Keystroke, ParentElement as _, SharedString, px};
use gpui_component::{
    button::{Button, ButtonVariants},
    kbd::Kbd,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectableState {
    Active,
    Inactive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionEmphasis {
    Primary,
    Secondary,
}

pub fn selectable_state_classes(state: SelectableState) -> &'static str {
    match state {
        SelectableState::Active => "selectable active",
        SelectableState::Inactive => "selectable inactive",
    }
}

pub fn workbench_action_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    shortcut: &'static str,
    emphasis: ActionEmphasis,
) -> Button {
    let button = Button::new(id)
        .label(label)
        .compact()
        .rounded(px(6.0))
        .child(Kbd::new(shortcut_keystroke(shortcut)));

    match emphasis {
        ActionEmphasis::Primary => button.primary(),
        ActionEmphasis::Secondary => button.outline(),
    }
}

fn shortcut_keystroke(shortcut: &str) -> Keystroke {
    Keystroke::parse(shortcut).expect("workbench shortcut should be a valid GPUI keystroke")
}
