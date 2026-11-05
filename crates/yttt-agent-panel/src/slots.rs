//! Places where the host adds its own controls. Each slot holds any number
//! of items, ordered by `order` and replaced by `id`, so several features
//! can contribute without knowing about each other. Renderers get a
//! read-only snapshot of the thread and return `None` to show nothing.
//!
//! Slots are for the host's own chrome. What the agent produces travels as
//! data instead ([`crate::event::Action`], [`crate::event::ToolContent::Custom`]),
//! since a renderer cannot cross from the agent's process to the panel's.
use std::{collections::HashMap, rc::Rc};

use gpui::{AnyElement, App, SharedString, Window};

use crate::{
    event::TodoItem,
    model::{ChangedFile, ThreadStatus, Usage},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    /// In the header, after the title.
    HeaderStart,
    /// In the header, before the new-thread button.
    HeaderEnd,
    /// The strip above the composer, among the built-in bars; see
    /// [`StatusItem`] for their orders.
    StatusBar,
    /// The composer's bottom row, after the built-in buttons and pickers.
    ComposerFooterStart,
    /// The composer's bottom row, before the context meter and send button.
    ComposerFooterEnd,
}

/// What a slot renderer can see of the thread.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SlotContext {
    pub status: ThreadStatus,
    pub usage: Option<Usage>,
    pub title: Option<String>,
    pub plan: Option<Vec<TodoItem>>,
    pub changed_files: Vec<ChangedFile>,
}

pub type SlotRender = Rc<dyn Fn(&SlotContext, &mut Window, &mut App) -> Option<AnyElement>>;

#[derive(Clone)]
pub struct SlotItem {
    pub id: SharedString,
    /// Lower comes first; items with equal order keep insertion order.
    pub order: i32,
    pub render: SlotRender,
}

impl SlotItem {
    pub fn new(
        id: impl Into<SharedString>,
        order: i32,
        render: impl Fn(&SlotContext, &mut Window, &mut App) -> Option<AnyElement> + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            order,
            render: Rc::new(render),
        }
    }
}

#[derive(Clone, Default)]
pub struct Slots {
    items: HashMap<Slot, Vec<SlotItem>>,
}

impl Slots {
    /// Adds `item`, replacing the one with the same id in that slot.
    pub fn insert(&mut self, slot: Slot, item: SlotItem) {
        let items = self.items.entry(slot).or_default();
        items.retain(|existing| existing.id != item.id);
        let at = items.partition_point(|existing| existing.order <= item.order);
        items.insert(at, item);
    }

    pub fn remove(&mut self, slot: Slot, id: &str) {
        if let Some(items) = self.items.get_mut(&slot) {
            items.retain(|item| item.id != id);
        }
    }

    pub fn items(&self, slot: Slot) -> &[SlotItem] {
        self.items.get(&slot).map_or(&[], Vec::as_slice)
    }

    /// Each item's element, in order, skipping those that render nothing.
    pub fn render(
        &self,
        slot: Slot,
        context: &SlotContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<AnyElement> {
        self.render_ordered(slot, context, window, cx)
            .into_iter()
            .map(|(_, element)| element)
            .collect()
    }

    /// Like [`Self::render`], keeping each element's order to merge with
    /// built-in content.
    pub fn render_ordered(
        &self,
        slot: Slot,
        context: &SlotContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<(i32, AnyElement)> {
        self.items(slot)
            .to_vec()
            .into_iter()
            .filter_map(|item| {
                (item.render)(context, window, cx).map(|element| (item.order, element))
            })
            .collect()
    }
}

/// Draws a [`crate::event::ToolContent::Custom`] payload (JSON text).
pub type CustomRender = Rc<dyn Fn(&str, &mut Window, &mut App) -> AnyElement>;

/// Where a host message action shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessagePlacement {
    /// On a prompt the user sent, beside Copy and Edit.
    User,
    /// On the line that closes a turn, beside Copy and Retry.
    TurnEnd,
}

/// A host button on messages, like "Fork from here" or "Helpful".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageAction {
    pub id: String,
    pub label: String,
    pub placement: MessagePlacement,
}

/// The built-in bars above the composer. Each can be hidden, and host bars
/// go between them by order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StatusItem {
    Activity,
    Plan,
    Changes,
    Queue,
    Editing,
}

impl StatusItem {
    pub const ALL: [Self; 5] = [
        Self::Activity,
        Self::Plan,
        Self::Changes,
        Self::Queue,
        Self::Editing,
    ];

    /// Spaced out so host bars fit between them.
    pub fn order(self) -> i32 {
        match self {
            Self::Activity => 100,
            Self::Plan => 200,
            Self::Changes => 300,
            Self::Queue => 400,
            Self::Editing => 500,
        }
    }
}

/// Sorts built-in and host elements by order; built-ins go first on ties.
pub fn merge_ordered(
    mut builtin: Vec<(i32, AnyElement)>,
    host: Vec<(i32, AnyElement)>,
) -> Vec<AnyElement> {
    builtin.extend(host);
    builtin.sort_by_key(|(order, _)| *order);
    builtin.into_iter().map(|(_, element)| element).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &'static str, order: i32) -> SlotItem {
        SlotItem::new(id, order, |_, _, _| None)
    }

    fn ids(slots: &Slots, slot: Slot) -> Vec<&str> {
        slots
            .items(slot)
            .iter()
            .map(|item| item.id.as_ref())
            .collect()
    }

    #[test]
    fn items_keep_order_then_insertion_order() {
        let mut slots = Slots::default();
        slots.insert(Slot::HeaderEnd, item("b", 10));
        slots.insert(Slot::HeaderEnd, item("a", 0));
        slots.insert(Slot::HeaderEnd, item("c", 10));
        assert_eq!(ids(&slots, Slot::HeaderEnd), ["a", "b", "c"]);
        assert!(slots.items(Slot::HeaderStart).is_empty());
    }

    #[test]
    fn the_same_id_replaces_and_remove_drops() {
        let mut slots = Slots::default();
        slots.insert(Slot::StatusBar, item("a", 0));
        slots.insert(Slot::StatusBar, item("b", 5));
        slots.insert(Slot::StatusBar, item("a", 9));
        assert_eq!(ids(&slots, Slot::StatusBar), ["b", "a"]);
        slots.remove(Slot::StatusBar, "b");
        assert_eq!(ids(&slots, Slot::StatusBar), ["a"]);
    }

    #[test]
    fn built_in_bars_leave_room_between_them() {
        let orders = StatusItem::ALL.map(StatusItem::order);
        assert!(orders.windows(2).all(|pair| pair[1] - pair[0] >= 100));
    }
}
