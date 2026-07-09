use crate::palette::{PaletteItem, PaletteKind};

use super::PickerItem;

pub trait PickerDelegate {
    fn items(&self) -> &[PickerItem];
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
            items: items.iter().map(PickerItem::from_palette_item).collect(),
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
