use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

use alacritty_terminal::{
    event::EventListener,
    graphics::{ColorType, GraphicData, MAX_GRAPHIC_ASSETS, MAX_GRAPHIC_BYTES},
    term::Term,
};
use parking_lot::Mutex;
use yttt_protocol::terminal::TerminalImage;

pub(crate) type SharedImages = Arc<Mutex<ImageStore>>;

#[derive(Default)]
pub(crate) struct ImageStore {
    images: BTreeMap<u64, Arc<TerminalImage>>,
    insertion_order: VecDeque<u64>,
    rgba_bytes: usize,
}

impl ImageStore {
    pub(crate) fn images(&self) -> &BTreeMap<u64, Arc<TerminalImage>> {
        &self.images
    }

    fn remove(&mut self, id: u64) -> bool {
        let Some(image) = self.images.remove(&id) else {
            return false;
        };

        self.rgba_bytes = self.rgba_bytes.saturating_sub(image.rgba.len());
        self.insertion_order.retain(|entry| *entry != id);
        true
    }

    fn insert(&mut self, image: TerminalImage, evicted_ids: &mut Vec<u64>) {
        let id = image.id;
        let rgba_bytes = image.rgba.len();
        if rgba_bytes > MAX_GRAPHIC_BYTES || self.images.contains_key(&id) {
            return;
        }

        while self.images.len() >= MAX_GRAPHIC_ASSETS
            || self.rgba_bytes > MAX_GRAPHIC_BYTES - rgba_bytes
        {
            let Some(id) = self.insertion_order.pop_front() else {
                return;
            };
            if let Some(evicted) = self.images.remove(&id) {
                self.rgba_bytes = self.rgba_bytes.saturating_sub(evicted.rgba.len());
                evicted_ids.push(id);
            }
        }

        self.rgba_bytes += rgba_bytes;
        self.insertion_order.push_back(id);
        self.images.insert(id, Arc::new(image));
    }
}

pub(crate) fn drain<L: EventListener>(term: &mut Term<L>, images: &mut ImageStore) {
    let mut evicted = Vec::new();
    let mut assets_disappeared = false;
    if let Some(queues) = term.graphics_take_queues() {
        for id in &queues.remove_queue {
            assets_disappeared |= images.remove(id.get());
        }
        for graphic in queues.pending {
            // Never resurrect an image erased within the same parser batch.
            if queues.remove_queue.contains(&graphic.id) {
                continue;
            }
            if let Some(image) = terminal_image(graphic) {
                images.insert(image, &mut evicted);
            }
        }
        // Sixel cell placements already describe the clipped regions.
        let _ = queues.clear_subregions;
    }

    loop {
        let updates = term.kitty_take_updates();
        if updates.images.is_empty() && updates.removed.is_empty() && evicted.is_empty() {
            break;
        }
        for id in &updates.removed {
            assets_disappeared |= images.remove(*id);
        }
        for image in updates.images {
            if updates.removed.contains(&image.id) {
                continue;
            }
            let (Ok(width), Ok(height)) = (u16::try_from(image.width), u16::try_from(image.height))
            else {
                term.kitty_forget_resource(image.id);
                continue;
            };
            images.insert(
                TerminalImage {
                    id: image.id,
                    width,
                    height,
                    rgba: image.rgba,
                },
                &mut evicted,
            );
        }
        for id in evicted.drain(..) {
            assets_disappeared = true;
            // The native animation registry shares these pixels; eviction must
            // release its owner and dependent placements as well as this map.
            term.kitty_forget_resource(id);
        }
    }
    if assets_disappeared {
        term.mark_fully_damaged();
    }
}

fn terminal_image(graphic: GraphicData) -> Option<TerminalImage> {
    let width = u16::try_from(graphic.width).ok()?;
    let height = u16::try_from(graphic.height).ok()?;
    let pixel_count = usize::from(width).checked_mul(usize::from(height))?;
    let rgba_bytes = pixel_count.checked_mul(4)?;
    if rgba_bytes > MAX_GRAPHIC_BYTES {
        return None;
    }

    let rgba = match graphic.color_type {
        ColorType::Rgba if graphic.pixels.len() == rgba_bytes => graphic.pixels,
        ColorType::Rgb if graphic.pixels.len() == pixel_count.checked_mul(3)? => {
            let mut rgba = Vec::with_capacity(rgba_bytes);
            for rgb in graphic.pixels.chunks_exact(3) {
                rgba.extend_from_slice(rgb);
                rgba.push(255);
            }
            rgba
        }
        _ => return None,
    };

    Some(TerminalImage {
        id: graphic.id.get(),
        width,
        height,
        rgba: Arc::new(rgba),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_oldest_assets_at_the_session_limit() {
        let mut store = ImageStore::default();
        let mut evicted = Vec::new();
        for id in 0..=MAX_GRAPHIC_ASSETS as u64 {
            store.insert(
                TerminalImage {
                    id,
                    width: 1,
                    height: 1,
                    rgba: Arc::new(vec![0; 4]),
                },
                &mut evicted,
            );
        }

        assert_eq!(store.images.len(), MAX_GRAPHIC_ASSETS);
        assert!(!store.images.contains_key(&0));
        assert!(store.images.contains_key(&(MAX_GRAPHIC_ASSETS as u64)));
    }

    #[test]
    fn image_erased_in_its_input_batch_does_not_retain_pixels() {
        let mut state = crate::TerminalState::new(8, 4, alacritty_terminal::event::VoidListener);
        state.process_bytes(b"\x1bPq#1;2;100;0;0!4~\x1b\\\x1b[H\x1b[2K");
        state.with_render_state(|_, images| assert!(images.is_empty()));
    }
}
