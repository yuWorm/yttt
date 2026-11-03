use crate::palette::{PaletteItem, PaletteKind};

use super::{PickerDelegate, PickerItem};

impl From<&PaletteItem> for PickerItem {
    fn from(item: &PaletteItem) -> Self {
        Self {
            id: item.id.clone(),
            title: item.title.clone(),
            subtitle: item.subtitle.clone(),
            status: item.status.clone(),
            keybinding: item.keybinding.clone(),
            enabled: item.enabled,
            disabled_reason: item.disabled_reason.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PalettePickerDelegate {
    kind: PaletteKind,
    items: Vec<PickerItem>,
}

impl PalettePickerDelegate {
    pub fn new(kind: PaletteKind, items: Vec<PaletteItem>) -> Self {
        Self {
            kind,
            items: items.iter().map(PickerItem::from).collect(),
        }
    }

    pub fn kind(&self) -> PaletteKind {
        self.kind
    }
}

impl PickerDelegate for PalettePickerDelegate {
    fn items(&self) -> &[PickerItem] {
        &self.items
    }
}
