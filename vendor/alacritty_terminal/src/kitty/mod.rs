//! State management for the Kitty graphics protocol.
//!
//! Decoder and animation storage deliberately live in separate modules.  This
//! module owns the protocol-visible image namespace, placements and resource
//! lifetime.  Pixel resource IDs are never Kitty image IDs: they are immutable,
//! globally unique handles consumed by the renderer.

pub mod animation;
pub mod decode;

use std::collections::HashSet;
use std::mem;
use std::time::Instant;

use crate::graphics::{MAX_GRAPHIC_ASSETS, MAX_GRAPHIC_BYTES};
use crate::grid::Grid;
use crate::index::{Column, Line};
use crate::term::cell::Cell;
use crate::vte::ansi::Color;

use self::animation::{FrameImage, Frames};
use self::decode::{Controls, DecodedCommand, DecodedImage, Decoder, ProtocolError};

const RESOURCE_NAMESPACE: u64 = 1 << 63;
const MAX_RELATIVE_DEPTH: usize = 32;
const PLACEHOLDER: char = '\u{10eeee}';

/// Geometry and cursor state at the time a Kitty command is processed.
#[derive(Copy, Clone, Debug)]
pub struct KittyContext {
    pub cursor_row: i32,
    pub cursor_column: usize,
    pub columns: usize,
    pub lines: usize,
    pub history_size: usize,
    pub cell_width: u16,
    pub cell_height: u16,
}

/// A single visible Kitty image placement.
///
/// The destination and clip rectangles use terminal pixels, relative to the
/// currently requested viewport.  Source coordinates always refer to the
/// immutable current frame resource named by `image_id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KittyPlacement {
    pub image_id: u64,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub source_x: u32,
    pub source_y: u32,
    pub source_width: u32,
    pub source_height: u32,
    pub clip_x: i32,
    pub clip_y: i32,
    pub clip_width: u32,
    pub clip_height: u32,
    pub z_index: i32,
    pub order: u32,
}

/// The resources and placements needed to render a Kitty viewport.
#[derive(Default, Clone, Debug)]
pub struct KittyScene {
    pub placements: Vec<KittyPlacement>,
    pub image_ids: Vec<u64>,
}

/// Pixel resource changes since the last call to [`KittyState::take_updates`].
#[derive(Default, Debug)]
pub struct KittyUpdates {
    pub images: Vec<FrameImage>,
    pub removed: Vec<u64>,
}

/// Result of processing one complete APC command.
#[derive(Default, Debug)]
pub struct KittyResult {
    pub replies: Vec<String>,
    pub cursor: Option<(i32, usize)>,
    pub changed: bool,
}

#[derive(Debug, Default)]
struct Buffer {
    images: Vec<Image>,
}

impl Buffer {
    fn bytes(&self) -> usize {
        self.images.iter().map(|image| image.frames.bytes()).sum()
    }

    fn frames_len(&self) -> usize {
        self.images.iter().map(|image| image.frames.len()).sum()
    }

    fn image_index(&self, id: u32) -> Option<usize> {
        (id != 0)
            .then(|| self.images.iter().position(|image| image.id == id))
            .flatten()
    }

    fn numbered_image_index(&self, number: u32) -> Option<usize> {
        (number != 0)
            .then(|| self.images.iter().rposition(|image| image.number == number))
            .flatten()
    }

    fn parent(&self, key: ParentKey) -> Option<(usize, usize)> {
        if key.image_id == 0 || key.placement_id == 0 {
            return None;
        }

        let image_index = self.image_index(key.image_id)?;
        self.images[image_index]
            .placements
            .iter()
            .position(|placement| placement.id == key.placement_id)
            .map(|placement_index| (image_index, placement_index))
    }

    fn virtual_placement(&self, image_id: u32, placement_id: u32) -> Option<(usize, usize)> {
        let image_index = self.image_index(image_id)?;
        let image = &self.images[image_index];

        // Missing underline color means that any virtual placement for this
        // image is acceptable. Prefer the unnumbered placement when present.
        let placement_index = if placement_id == 0 {
            image
                .placements
                .iter()
                .position(|placement| placement.id == 0 && placement.is_virtual())
                .or_else(|| {
                    image
                        .placements
                        .iter()
                        .position(StoredPlacement::is_virtual)
                })?
        } else {
            image
                .placements
                .iter()
                .position(|placement| placement.id == placement_id && placement.is_virtual())?
        };

        Some((image_index, placement_index))
    }
}

#[derive(Debug)]
struct Image {
    /// Client-visible Kitty image ID.  Zero is an anonymous image.
    id: u32,
    /// Client-visible Kitty image number, if this image was created with `I`.
    number: u32,
    transient: bool,
    frames: Frames,
    placements: Vec<StoredPlacement>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ParentKey {
    image_id: u32,
    placement_id: u32,
}

#[derive(Clone, Copy, Debug)]
struct SourceRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy, Debug)]
enum Destination {
    Natural,
    Cells { columns: u32, rows: u32 },
    WidthCells(u32),
    HeightCells(u32),
}

#[derive(Clone, Copy, Debug)]
enum Anchor {
    Cursor {
        column: i32,
        row: i32,
        offset_x: i32,
        offset_y: i32,
    },
    Relative {
        parent: ParentKey,
        horizontal: i32,
        vertical: i32,
        offset_x: i32,
        offset_y: i32,
    },
}

#[derive(Clone, Copy, Debug)]
enum PlacementKind {
    Physical {
        anchor: Anchor,
        destination: Destination,
        /// The vertical range, in buffer rows, which survived a partial scroll.
        clip_rows: Option<(i32, i32)>,
        /// Conservative span used by the scroll hook, which intentionally has
        /// no pixel-geometry parameter.
        row_span: u32,
    },
    Virtual {
        columns: u32,
        rows: u32,
    },
}

#[derive(Clone, Debug)]
struct StoredPlacement {
    id: u32,
    source: SourceRect,
    z_index: i32,
    kind: PlacementKind,
}

impl StoredPlacement {
    fn is_virtual(&self) -> bool {
        matches!(self.kind, PlacementKind::Virtual { .. })
    }

    fn parent_key(&self, image_id: u32) -> Option<ParentKey> {
        (image_id != 0 && self.id != 0).then_some(ParentKey {
            image_id,
            placement_id: self.id,
        })
    }

    fn relative_parent(&self) -> Option<ParentKey> {
        match self.kind {
            PlacementKind::Physical {
                anchor: Anchor::Relative { parent, .. },
                ..
            } => Some(parent),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ResolvedGeometry {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    clip_rows: Option<(i32, i32)>,
}

#[derive(Clone, Copy, Debug)]
struct VirtualAnchor {
    x: i32,
    y: i32,
}

/// Complete state for one terminal's Kitty graphics protocol.
#[derive(Debug)]
pub struct KittyState {
    decoder: Decoder,
    primary: Buffer,
    alternate: Buffer,
    alternate_active: bool,
    next_resource: u64,
    next_client_id: u32,
    file_transfers: bool,
    pending_images: Vec<FrameImage>,
    pending_image_ids: HashSet<u64>,
    pending_removed: Vec<u64>,
    pending_removed_ids: HashSet<u64>,
}

impl Default for KittyState {
    fn default() -> Self {
        Self {
            decoder: Decoder::default(),
            primary: Buffer::default(),
            alternate: Buffer::default(),
            alternate_active: false,
            next_resource: 1,
            next_client_id: 1,
            file_transfers: true,
            pending_images: Vec::new(),
            pending_image_ids: HashSet::new(),
            pending_removed: Vec::new(),
            pending_removed_ids: HashSet::new(),
        }
    }
}

impl KittyState {
    /// Process one APC payload. `bytes` starts with the Kitty `G` introducer.
    pub fn handle_apc(
        &mut self,
        bytes: &[u8],
        context: KittyContext,
        grid: &Grid<Cell>,
        now: Instant,
    ) -> KittyResult {
        let decoded = self.decoder.decode(bytes, self.file_transfers);
        match decoded {
            Ok(None) => KittyResult::default(),
            Err(error) => self.error_result(error),
            Ok(Some(command)) => self.dispatch(command, context, grid, now),
        }
    }

    /// Abort a chunked direct transfer without producing a protocol response.
    pub fn abort_transfer(&mut self) {
        self.decoder.abort();
    }

    /// Return all resource changes and clear the coalescing queues.
    pub fn take_updates(&mut self) -> KittyUpdates {
        self.pending_image_ids.clear();
        self.pending_removed_ids.clear();
        KittyUpdates {
            images: mem::take(&mut self.pending_images),
            removed: mem::take(&mut self.pending_removed),
        }
    }

    /// Forget a renderer resource after canonical-store eviction.
    pub fn forget_resource(&mut self, id: u64) -> bool {
        for alternate in [false, true] {
            let removed = {
                let buffer = if alternate {
                    &mut self.alternate
                } else {
                    &mut self.primary
                };
                buffer
                    .images
                    .iter()
                    .position(|image| image.frames.images().any(|frame| frame.id == id))
                    .map(|index| take_image(buffer, index))
            };
            let Some((frame_ids, parent_keys)) = removed else {
                continue;
            };
            let active = self.alternate_active;
            self.alternate_active = alternate;
            if self.pending_image_ids.remove(&id) {
                if let Some(index) = self.pending_images.iter().position(|image| image.id == id) {
                    self.pending_images.remove(index);
                }
            }
            for frame_id in frame_ids {
                if frame_id != id {
                    self.queue_removed(frame_id);
                }
            }
            self.remove_descendants_from(parent_keys);
            self.alternate_active = active;
            return true;
        }
        false
    }

    /// Advance visible animations for the active screen.
    pub fn tick(&mut self, now: Instant) -> bool {
        self.buffer_mut()
            .images
            .iter_mut()
            .filter(|image| !image.placements.is_empty())
            .any(|image| image.frames.tick(now))
    }

    /// Return the next active animation deadline.
    pub fn deadline(&self) -> Option<Instant> {
        self.buffer()
            .images
            .iter()
            .filter(|image| !image.placements.is_empty())
            .filter_map(|image| image.frames.deadline())
            .min()
    }

    /// Apply a terminal scroll operation to direct placements.
    pub fn scroll(
        &mut self,
        start: i32,
        end: i32,
        delta: i32,
        history_size: usize,
        screen_lines: usize,
    ) {
        if delta == 0 || start >= end {
            return;
        }
        let full_primary_upward_scroll =
            !self.alternate_active && start == 0 && end as usize == screen_lines && delta < 0;
        let history_top = -(history_size.min(i32::MAX as usize) as i32);
        let mut remove = Vec::new();
        for (image_index, image) in self.buffer_mut().images.iter_mut().enumerate() {
            for (placement_index, placement) in image.placements.iter_mut().enumerate() {
                let PlacementKind::Physical {
                    anchor: Anchor::Cursor { row, .. },
                    clip_rows,
                    row_span,
                    ..
                } = &mut placement.kind
                else {
                    continue;
                };
                let span = (*row_span).clamp(1, i32::MAX as u32) as i32;
                let bottom = row.saturating_add(span);
                let (visible_top, visible_bottom) = clip_rows
                    .map_or((*row, bottom), |(low, high)| {
                        ((*row).max(low), bottom.min(high))
                    });
                if full_primary_upward_scroll {
                    // Existing history scrolls too; a negative origin must not
                    // strand the image on its first scrollback line.
                    if visible_top >= end {
                        continue;
                    }
                } else if visible_top < start || visible_bottom > end {
                    continue;
                }
                *row = row.saturating_add(delta);
                let low = visible_top.saturating_add(delta);
                let high = visible_bottom.saturating_add(delta);
                let allowed_top = if full_primary_upward_scroll {
                    history_top
                } else {
                    start
                };
                let allowed_bottom = if full_primary_upward_scroll {
                    i32::MAX
                } else {
                    end
                };
                let clipped = (low.max(allowed_top), high.min(allowed_bottom));
                *clip_rows = if clip_rows.is_some() || clipped != (low, high) {
                    Some(clipped)
                } else {
                    None
                };
                if clipped.0 >= clipped.1 {
                    remove.push((image_index, placement_index));
                }
            }
        }
        self.remove_selected_placements(remove, false);
    }

    /// Update placement row coordinates after resizing the active terminal grid.
    pub fn resize(&mut self, context: KittyContext, row_delta: i32) {
        self.resize_buffer(self.alternate_active, context, row_delta);
    }

    /// Update placement row coordinates for the inactive grid.  This keeps
    /// primary scrollback independent from an active alternate screen.
    pub fn resize_inactive(&mut self, context: KittyContext, row_delta: i32) {
        self.resize_buffer(!self.alternate_active, context, row_delta);
    }

    fn resize_buffer(&mut self, alternate: bool, context: KittyContext, row_delta: i32) {
        let active = self.alternate_active;
        self.alternate_active = alternate;
        let mut remove = Vec::new();
        {
            let buffer = self.buffer_mut();
            for (image_index, image) in buffer.images.iter_mut().enumerate() {
                for (placement_index, placement) in image.placements.iter_mut().enumerate() {
                    let PlacementKind::Physical {
                        anchor: Anchor::Cursor { row, offset_y, .. },
                        clip_rows,
                        row_span,
                        destination,
                    } = &mut placement.kind
                    else {
                        continue;
                    };

                    let (_, height, _, rows) =
                        destination_pixels(*destination, placement.source, context);
                    *row_span = rows.max(ceil_div(
                        ((*offset_y).max(0) as u32).saturating_add(height),
                        u32::from(context.cell_height.max(1)),
                    ));
                    *row = row.saturating_add(row_delta);
                    let bottom = row.saturating_add((*row_span).clamp(1, i32::MAX as u32) as i32);
                    if bottom <= -(context.history_size as i32) {
                        remove.push((image_index, placement_index));
                    } else if let Some((low, high)) = clip_rows {
                        *low = low
                            .saturating_add(row_delta)
                            .max(-(context.history_size as i32));
                        *high = high.saturating_add(row_delta).min(context.lines as i32);
                        if *low >= *high {
                            remove.push((image_index, placement_index));
                        }
                    }
                }
            }
        }
        self.remove_selected_placements(remove, false);
        self.alternate_active = active;
    }

    /// Enter or leave the alternate screen. Entering always starts with an empty
    /// alternate Kitty store; the primary store remains untouched.
    pub fn switch_screen(&mut self, alternate: bool) {
        if alternate && !self.alternate_active {
            let ids = remove_all_images(&mut self.alternate);
            for id in ids {
                self.queue_removed(id);
            }
        }
        self.alternate_active = alternate;
    }

    /// Clear direct placements visible in the current viewport while retaining
    /// their history-only portions.
    pub fn clear_visible(&mut self, context: KittyContext) {
        let mut remove = Vec::new();
        {
            let buffer = self.buffer_mut();
            for (image_index, image) in buffer.images.iter_mut().enumerate() {
                for (placement_index, placement) in image.placements.iter_mut().enumerate() {
                    let PlacementKind::Physical {
                        anchor: Anchor::Cursor { row, .. },
                        clip_rows,
                        row_span,
                        ..
                    } = &mut placement.kind
                    else {
                        continue;
                    };
                    let bottom = row.saturating_add((*row_span).clamp(1, i32::MAX as u32) as i32);
                    if bottom <= 0 || *row >= context.lines as i32 {
                        continue;
                    }
                    if *row < 0 {
                        *clip_rows = intersect_rows(*clip_rows, (i32::MIN, 0));
                    } else {
                        remove.push((image_index, placement_index));
                    }
                }
            }
        }
        self.remove_selected_placements(remove, false);
    }

    /// Remove direct-placement history while retaining portions still visible.
    pub fn clear_history(&mut self) {
        let buffer = self.buffer_mut();
        let mut remove = Vec::new();
        for (image_index, image) in buffer.images.iter_mut().enumerate() {
            for (placement_index, placement) in image.placements.iter_mut().enumerate() {
                let PlacementKind::Physical {
                    anchor: Anchor::Cursor { row, .. },
                    clip_rows,
                    row_span,
                    ..
                } = &mut placement.kind
                else {
                    continue;
                };
                let bottom = row.saturating_add((*row_span).clamp(1, i32::MAX as u32) as i32);
                if bottom <= 0 {
                    remove.push((image_index, placement_index));
                } else if *row < 0 {
                    *clip_rows = intersect_rows(*clip_rows, (0, i32::MAX));
                }
            }
        }
        self.remove_selected_placements(remove, false);
    }

    /// Clear both stores and all pending decoder state, preserving resource-ID
    /// monotonicity for the lifetime of the terminal.
    pub fn reset(&mut self) {
        self.decoder.abort();
        let primary = remove_all_images(&mut self.primary);
        let alternate = remove_all_images(&mut self.alternate);
        for id in primary.into_iter().chain(alternate) {
            self.queue_removed(id);
        }
        self.alternate_active = false;
    }

    /// Enable or disable filename/shared-memory transports (disabled for remote
    /// PTY streams where terminal-local paths are not trusted).
    pub fn set_file_transfers(&mut self, allowed: bool) {
        self.file_transfers = allowed;
    }

    /// Construct the renderer-facing scene for a particular viewport.
    pub fn scene(
        &self,
        grid: &Grid<Cell>,
        display_offset: usize,
        context: KittyContext,
    ) -> KittyScene {
        let buffer = self.buffer();
        let (mut placements, anchors, mut image_ids) =
            virtual_placements(buffer, grid, display_offset, context);

        for (image_index, image) in buffer.images.iter().enumerate() {
            for (placement_index, placement) in image.placements.iter().enumerate() {
                if placement.is_virtual() {
                    continue;
                }
                let mut seen = Vec::new();
                let Some(mut geometry) = resolve_geometry(
                    buffer,
                    image_index,
                    placement_index,
                    &anchors,
                    context,
                    &mut seen,
                    0,
                ) else {
                    continue;
                };
                let viewport_rows = display_offset.min(i32::MAX as usize) as i32;
                geometry.y = geometry
                    .y
                    .saturating_add(viewport_rows.saturating_mul(context.cell_height as i32));
                geometry.clip_rows = geometry.clip_rows.map(|(low, high)| {
                    (
                        low.saturating_add(viewport_rows),
                        high.saturating_add(viewport_rows),
                    )
                });

                if let Some(output) = clipped_output(image, placement, geometry, context) {
                    append_image_resources(&mut image_ids, &image.frames);
                    placements.push(output);
                }
            }
        }

        placements.sort_by_key(|placement| (placement.z_index, placement.order));
        image_ids.sort_unstable();
        image_ids.dedup();
        KittyScene {
            placements,
            image_ids,
        }
    }

    fn dispatch(
        &mut self,
        command: DecodedCommand,
        context: KittyContext,
        grid: &Grid<Cell>,
        now: Instant,
    ) -> KittyResult {
        let action = command.controls.byte(b'a', b't');
        let controls = command.controls;
        if controls.unsigned(b'i', 0) != 0 && controls.unsigned(b'I', 0) != 0 {
            return self.error_result(protocol_error(
                "EINVAL",
                "i and I cannot be used together",
                &controls,
            ));
        }
        let result = match action {
            b't' | b'T' => match command.image {
                Some(image) => self.transmit(&controls, image, action == b'T', context),
                None => Err(protocol_error(
                    "EINVAL",
                    "transmit is missing image data",
                    &controls,
                )),
            },
            b'q' => {
                if command.image.is_some() {
                    Ok(Dispatch {
                        image_id: controls.unsigned(b'i', 0),
                        image_number: controls.unsigned(b'I', 0),
                        placement_id: 0,
                        cursor: None,
                        changed: false,
                    })
                } else {
                    Err(protocol_error(
                        "EINVAL",
                        "query is missing image data",
                        &controls,
                    ))
                }
            }
            b'p' => self.put(&controls, context),
            b'd' => self.delete(&controls, context, grid, now),
            b'f' => match command.image {
                Some(image) => self.transmit_frame(&controls, image, now),
                None => Err(protocol_error(
                    "EINVAL",
                    "frame transmit is missing image data",
                    &controls,
                )),
            },
            b'a' => self.control_animation(&controls, now),
            b'c' => self.compose_animation(&controls, now),
            _ => Err(protocol_error(
                "EINVAL",
                "unsupported Kitty graphics action",
                &controls,
            )),
        };

        match result {
            Ok(dispatch) => self.success_result(&controls, dispatch),
            Err(error) => self.error_result(error),
        }
    }

    fn transmit(
        &mut self,
        controls: &Controls,
        decoded: DecodedImage,
        place: bool,
        context: KittyContext,
    ) -> Result<Dispatch, ProtocolError> {
        let (id, number) = self.new_image_identity(controls)?;
        let frame_id = self.next_resource_id(controls)?;
        let frame = FrameImage {
            id: frame_id,
            width: decoded.width,
            height: decoded.height,
            rgba: decoded.rgba,
        };

        let mut pending_placement = if place {
            Some(build_placement(controls, &frame, context)?)
        } else {
            None
        };
        if id == 0 {
            if let Some(placement) = &mut pending_placement {
                placement.id = 0;
            }
        }
        if let Some(placement) = &pending_placement {
            validate_parent(
                self.buffer(),
                placement,
                id,
                controls.unsigned(b'i', 0),
                controls,
            )?;
        }

        let replacing = (id != 0).then_some(id);
        self.make_room_for_root(replacing, frame.rgba.len(), 1, controls)?;
        if let Some(old_id) = replacing {
            self.remove_image_id(old_id, true);
        }

        let image_index = self.buffer_mut().images.len();
        self.buffer_mut().images.push(Image {
            id,
            number,
            transient: controls.unsigned(b'N', 0) & 1 != 0,
            frames: Frames::new(frame.clone()),
            placements: Vec::new(),
        });
        self.queue_image(frame);

        let (placement_id, cursor) = if let Some(placement) = pending_placement {
            let cursor = cursor_after(&placement, context, controls.unsigned(b'C', 0) == 0);
            let placement_id = placement.id;
            insert_placement(&mut self.buffer_mut().images[image_index], placement);
            (placement_id, cursor)
        } else {
            (0, None)
        };

        Ok(Dispatch {
            image_id: id,
            image_number: number,
            placement_id,
            cursor,
            changed: true,
        })
    }

    fn put(
        &mut self,
        controls: &Controls,
        context: KittyContext,
    ) -> Result<Dispatch, ProtocolError> {
        let index = self.find_image(controls)?;
        let (id, number, frame) = {
            let image = &self.buffer().images[index];
            (image.id, image.number, image.frames.current().clone())
        };
        let placement = build_placement(controls, &frame, context)?;
        validate_parent(self.buffer(), &placement, id, 0, controls)?;
        let cursor = cursor_after(&placement, context, controls.unsigned(b'C', 0) == 0);
        let placement_id = placement.id;
        insert_placement(&mut self.buffer_mut().images[index], placement);
        Ok(Dispatch {
            image_id: id,
            image_number: number,
            placement_id,
            cursor,
            changed: true,
        })
    }

    fn delete(
        &mut self,
        controls: &Controls,
        context: KittyContext,
        grid: &Grid<Cell>,
        now: Instant,
    ) -> Result<Dispatch, ProtocolError> {
        self.decoder.abort();
        let selector = controls.byte(b'd', b'a');
        if matches!(selector, b'f' | b'F') {
            return self.delete_frames(controls, now);
        }

        let upper = selector.is_ascii_uppercase();
        let selector = selector.to_ascii_lowercase();
        let mut selected = Vec::new();
        let mut remove_images = Vec::new();

        match selector {
            b'a' => selected = self.physical_indices_in_rect(grid, context, None, None, None),
            b'i' | b'n' => {
                let image = if selector == b'i' {
                    self.find_image(controls)?
                } else {
                    let number = controls.unsigned(b'I', 0);
                    self.buffer().numbered_image_index(number).ok_or_else(|| {
                        protocol_error("ENOENT", "image number was not found", controls)
                    })?
                };
                let placement_id = controls.unsigned(b'p', 0);
                if placement_id != 0 {
                    if let Some(placement) = self.buffer().images[image]
                        .placements
                        .iter()
                        .position(|placement| placement.id == placement_id)
                    {
                        selected.push((image, placement));
                    }
                } else if upper {
                    remove_images.push(image);
                } else {
                    selected.extend(
                        (0..self.buffer().images[image].placements.len())
                            .map(|placement| (image, placement)),
                    );
                }
            }
            b'r' => {
                let start = controls.unsigned(b'x', 0);
                let end = controls.unsigned(b'y', u32::MAX);
                for (image, stored) in self.buffer().images.iter().enumerate() {
                    if stored.id < start || stored.id > end {
                        continue;
                    }
                    if upper {
                        remove_images.push(image);
                    } else {
                        selected.extend(
                            (0..stored.placements.len()).map(|placement| (image, placement)),
                        );
                    }
                }
            }
            b'c' => {
                selected = self.physical_indices_in_rect(
                    grid,
                    context,
                    Some(context.cursor_column as i32 + 1),
                    Some(context.cursor_row + 1),
                    None,
                );
            }
            b'p' | b'q' => {
                let column = required_cell_coordinate(controls, b'x')?;
                let row = required_cell_coordinate(controls, b'y')?;
                let z = (selector == b'q').then(|| controls.signed(b'z', 0));
                selected = self.physical_indices_in_rect(grid, context, Some(column), Some(row), z);
            }
            b'x' => {
                let column = required_cell_coordinate(controls, b'x')?;
                selected = self.physical_indices_in_rect(grid, context, Some(column), None, None);
            }
            b'y' => {
                let row = required_cell_coordinate(controls, b'y')?;
                selected = self.physical_indices_in_rect(grid, context, None, Some(row), None);
            }
            b'z' => {
                let z = controls.signed(b'z', 0);
                selected.extend(self.buffer().images.iter().enumerate().flat_map(
                    |(image_index, image)| {
                        image.placements.iter().enumerate().filter_map(
                            move |(placement_index, placement)| {
                                (!placement.is_virtual() && placement.z_index == z)
                                    .then_some((image_index, placement_index))
                            },
                        )
                    },
                ));
            }
            _ => {
                return Err(protocol_error(
                    "EINVAL",
                    "unsupported delete selector",
                    controls,
                ))
            }
        }

        let changed = !remove_images.is_empty() || !selected.is_empty();
        remove_images.sort_unstable();
        remove_images.dedup();
        for index in remove_images.into_iter().rev() {
            self.remove_image_at(index, true);
        }
        self.remove_selected_placements(selected, upper);
        if upper {
            self.remove_orphan_images();
        }

        Ok(Dispatch {
            image_id: controls.unsigned(b'i', 0),
            image_number: controls.unsigned(b'I', 0),
            placement_id: controls.unsigned(b'p', 0),
            cursor: None,
            changed,
        })
    }

    fn transmit_frame(
        &mut self,
        controls: &Controls,
        decoded: DecodedImage,
        now: Instant,
    ) -> Result<Dispatch, ProtocolError> {
        let initial_index = self.find_image(controls)?;
        let target_id = self.buffer().images[initial_index].id;
        let appending = {
            let image = &self.buffer().images[initial_index];
            let target = controls.unsigned(b'r', 0) as usize;
            target == 0 || target > image.frames.len()
        };
        if appending
            && (self.buffer().frames_len() >= MAX_GRAPHIC_ASSETS
                || self.buffer().images[initial_index].frames.len() >= MAX_GRAPHIC_ASSETS)
        {
            return Err(protocol_error(
                "ENOSPC",
                "Kitty graphics frame quota exceeded",
                controls,
            ));
        }
        let resource_id = self.next_resource_id(controls)?;

        let (id, number, before, after) = loop {
            let index = self
                .buffer()
                .image_index(target_id)
                .ok_or_else(|| protocol_error("ENOENT", "animation image was evicted", controls))?;
            let byte_limit = self.available_frame_bytes(index);
            let before = frame_images(&self.buffer().images[index].frames);
            let attempt = self.buffer_mut().images[index].frames.transmit(
                controls,
                decoded.clone(),
                resource_id,
                now,
                byte_limit,
            );
            match attempt {
                Ok(()) => {
                    let image = &self.buffer().images[index];
                    break (image.id, image.number, before, frame_images(&image.frames));
                }
                Err(error) if error.code == "ENOSPC" && self.evict_one_unplaced(target_id) => {}
                Err(error) => return Err(error),
            }
        };
        self.reconcile_frame_resources(before, after);
        Ok(Dispatch {
            image_id: id,
            image_number: number,
            placement_id: 0,
            cursor: None,
            changed: true,
        })
    }

    fn control_animation(
        &mut self,
        controls: &Controls,
        now: Instant,
    ) -> Result<Dispatch, ProtocolError> {
        let index = self.find_image(controls)?;
        let (id, number, before, after) = {
            let image = &mut self.buffer_mut().images[index];
            let before = frame_images(&image.frames);
            image.frames.control(controls, now)?;
            let after = frame_images(&image.frames);
            (image.id, image.number, before, after)
        };
        self.reconcile_frame_resources(before, after);
        Ok(Dispatch {
            image_id: id,
            image_number: number,
            placement_id: 0,
            cursor: None,
            changed: true,
        })
    }

    fn compose_animation(
        &mut self,
        controls: &Controls,
        now: Instant,
    ) -> Result<Dispatch, ProtocolError> {
        let index = self.find_image(controls)?;
        let resource_id = self.next_resource_id(controls)?;
        let byte_limit = self.available_frame_bytes(index);
        let (id, number, before, after) = {
            let image = &mut self.buffer_mut().images[index];
            let before = frame_images(&image.frames);
            image
                .frames
                .compose(controls, resource_id, now, byte_limit)?;
            let after = frame_images(&image.frames);
            (image.id, image.number, before, after)
        };
        self.reconcile_frame_resources(before, after);
        Ok(Dispatch {
            image_id: id,
            image_number: number,
            placement_id: 0,
            cursor: None,
            changed: true,
        })
    }

    fn delete_frames(
        &mut self,
        controls: &Controls,
        now: Instant,
    ) -> Result<Dispatch, ProtocolError> {
        let index = self.find_image(controls)?;
        let (id, number, empty, before, after) = {
            let image = &mut self.buffer_mut().images[index];
            let before = frame_images(&image.frames);
            image.frames.delete(controls, now)?;
            let empty = image.frames.len() == 0;
            let after = frame_images(&image.frames);
            (image.id, image.number, empty, before, after)
        };
        let changed = before
            .iter()
            .map(|frame| frame.id)
            .ne(after.iter().map(|frame| frame.id));
        self.reconcile_frame_resources(before, after);
        if empty {
            self.remove_image_id(id, true);
        }
        Ok(Dispatch {
            image_id: id,
            image_number: number,
            placement_id: 0,
            cursor: None,
            changed,
        })
    }

    fn find_image(&self, controls: &Controls) -> Result<usize, ProtocolError> {
        let id = controls.unsigned(b'i', 0);
        let number = controls.unsigned(b'I', 0);
        if id != 0 && number != 0 {
            return Err(protocol_error(
                "EINVAL",
                "i and I cannot be used together",
                controls,
            ));
        }
        let index = if id != 0 {
            self.buffer().image_index(id)
        } else {
            self.buffer().numbered_image_index(number)
        };
        index.ok_or_else(|| protocol_error("ENOENT", "image was not found", controls))
    }

    fn new_image_identity(&mut self, controls: &Controls) -> Result<(u32, u32), ProtocolError> {
        let requested_id = controls.unsigned(b'i', 0);
        let number = controls.unsigned(b'I', 0);
        if requested_id != 0 && number != 0 {
            return Err(protocol_error(
                "EINVAL",
                "i and I cannot be used together",
                controls,
            ));
        }
        if number == 0 {
            return Ok((requested_id, 0));
        }

        for _ in 0..u32::MAX {
            let id = self.next_client_id.max(1);
            self.next_client_id = id.wrapping_add(1).max(1);
            if self.buffer().image_index(id).is_none() {
                return Ok((id, number));
            }
        }
        Err(protocol_error(
            "ENOSPC",
            "no Kitty image identifiers remain",
            controls,
        ))
    }

    fn next_resource_id(&mut self, controls: &Controls) -> Result<u64, ProtocolError> {
        if self.next_resource >= RESOURCE_NAMESPACE {
            return Err(protocol_error(
                "ENOSPC",
                "Kitty resource identifiers are exhausted",
                controls,
            ));
        }
        let id = RESOURCE_NAMESPACE | self.next_resource;
        self.next_resource += 1;
        Ok(id)
    }

    fn available_frame_bytes(&self, target: usize) -> usize {
        let buffer = self.buffer();
        MAX_GRAPHIC_BYTES.saturating_sub(
            buffer
                .bytes()
                .saturating_sub(buffer.images[target].frames.bytes()),
        )
    }

    fn evict_one_unplaced(&mut self, except_image: u32) -> bool {
        let candidate = self
            .buffer()
            .images
            .iter()
            .enumerate()
            .filter(|(_, image)| image.id != except_image && image.placements.is_empty())
            .min_by_key(|(_, image)| !image.transient)
            .map(|(index, _)| index);
        let Some(index) = candidate else {
            return false;
        };
        self.remove_image_at(index, true);
        true
    }

    fn make_room_for_root(
        &mut self,
        replacing: Option<u32>,
        bytes: usize,
        frames: usize,
        controls: &Controls,
    ) -> Result<(), ProtocolError> {
        let buffer = self.buffer();
        let mut used_bytes = buffer.bytes();
        let mut used_frames = buffer.frames_len();
        if let Some(id) = replacing {
            if let Some(index) = buffer.image_index(id) {
                used_bytes = used_bytes.saturating_sub(buffer.images[index].frames.bytes());
                used_frames = used_frames.saturating_sub(buffer.images[index].frames.len());
            }
        }

        let mut evict = buffer
            .images
            .iter()
            .enumerate()
            .filter(|(_, image)| image.placements.is_empty() && Some(image.id) != replacing)
            .map(|(index, image)| {
                (
                    index,
                    image.transient,
                    image.frames.bytes(),
                    image.frames.len(),
                )
            })
            .collect::<Vec<_>>();
        evict.sort_by_key(|(_, transient, _, _)| !*transient);
        let mut selected = Vec::new();
        for (index, _, image_bytes, image_frames) in evict {
            if used_bytes.saturating_add(bytes) <= MAX_GRAPHIC_BYTES
                && used_frames.saturating_add(frames) <= MAX_GRAPHIC_ASSETS
            {
                break;
            }
            used_bytes = used_bytes.saturating_sub(image_bytes);
            used_frames = used_frames.saturating_sub(image_frames);
            selected.push(index);
        }
        if used_bytes.saturating_add(bytes) > MAX_GRAPHIC_BYTES
            || used_frames.saturating_add(frames) > MAX_GRAPHIC_ASSETS
        {
            return Err(protocol_error(
                "ENOSPC",
                "Kitty graphics storage quota exceeded",
                controls,
            ));
        }

        for index in selected.into_iter().rev() {
            self.remove_image_at(index, true);
        }
        Ok(())
    }

    fn remove_image_id(&mut self, id: u32, force_data_removal: bool) {
        if let Some(index) = self.buffer().image_index(id) {
            self.remove_image_at(index, force_data_removal);
        }
    }

    fn remove_image_at(&mut self, index: usize, _force_data_removal: bool) {
        let (ids, parent_keys) = take_image(self.buffer_mut(), index);
        for id in ids {
            self.queue_removed(id);
        }
        self.remove_descendants_from(parent_keys);
    }

    fn remove_orphan_images(&mut self) {
        let mut indexes = self
            .buffer()
            .images
            .iter()
            .enumerate()
            .filter_map(|(index, image)| image.placements.is_empty().then_some(index))
            .collect::<Vec<_>>();
        while let Some(index) = indexes.pop() {
            self.remove_image_at(index, true);
        }
    }

    fn remove_selected_placements(
        &mut self,
        mut selected: Vec<(usize, usize)>,
        free_orphans: bool,
    ) {
        if selected.is_empty() {
            return;
        }
        selected.sort_unstable();
        selected.dedup();
        let mut keys = Vec::new();
        for (image_index, placement_index) in selected.into_iter().rev() {
            let Some(image) = self.buffer_mut().images.get_mut(image_index) else {
                continue;
            };
            if placement_index >= image.placements.len() {
                continue;
            }
            let placement = image.placements.remove(placement_index);
            if let Some(key) = placement.parent_key(image.id) {
                keys.push(key);
            }
        }
        self.remove_descendants_from(keys);
        if free_orphans {
            self.remove_orphan_images();
        }
    }

    fn remove_descendants_from(&mut self, mut keys: Vec<ParentKey>) {
        let mut removed_image_candidates = HashSet::new();
        while !keys.is_empty() {
            let key_set = keys.drain(..).collect::<HashSet<_>>();
            let mut next = Vec::new();
            let buffer = self.buffer_mut();
            for image in &mut buffer.images {
                let mut placement_index = 0;
                while placement_index < image.placements.len() {
                    if image.placements[placement_index]
                        .relative_parent()
                        .is_some_and(|parent| key_set.contains(&parent))
                    {
                        let removed = image.placements.remove(placement_index);
                        if let Some(key) = removed.parent_key(image.id) {
                            next.push(key);
                        }
                        removed_image_candidates.insert(image.id);
                    } else {
                        placement_index += 1;
                    }
                }
            }
            keys = next;
        }

        // A relative placement's image lifetime is tied to its parent group.
        // Only images actually affected by cascading are eligible here; ordinary
        // lower-case delete commands still retain their unplaced image data.
        let mut candidates = removed_image_candidates.into_iter().collect::<Vec<_>>();
        candidates.sort_unstable();
        for id in candidates {
            if let Some(index) = self
                .buffer()
                .image_index(id)
                .filter(|&index| self.buffer().images[index].placements.is_empty())
            {
                self.remove_image_at(index, true);
            }
        }
    }

    fn reconcile_frame_resources(&mut self, before: Vec<FrameImage>, after: Vec<FrameImage>) {
        let before_ids = before.iter().map(|frame| frame.id).collect::<HashSet<_>>();
        let after_ids = after.iter().map(|frame| frame.id).collect::<HashSet<_>>();
        for frame in after {
            if !before_ids.contains(&frame.id) {
                self.queue_image(frame);
            }
        }
        for frame in before {
            if !after_ids.contains(&frame.id) {
                self.queue_removed(frame.id);
            }
        }
    }

    fn queue_image(&mut self, image: FrameImage) {
        if self.pending_removed_ids.contains(&image.id) || !self.pending_image_ids.insert(image.id)
        {
            return;
        }
        self.pending_images.push(image);
    }

    fn queue_removed(&mut self, id: u64) {
        if self.pending_image_ids.remove(&id) {
            if let Some(index) = self.pending_images.iter().position(|image| image.id == id) {
                self.pending_images.remove(index);
            }
            return;
        }
        if self.pending_removed_ids.insert(id) {
            self.pending_removed.push(id);
        }
    }

    fn success_result(&self, controls: &Controls, dispatch: Dispatch) -> KittyResult {
        let mut result = KittyResult {
            cursor: dispatch.cursor,
            changed: dispatch.changed,
            ..KittyResult::default()
        };
        let addressed = dispatch.image_id != 0
            || dispatch.image_number != 0
            || controls.byte(b'a', b't') == b'q';
        if addressed && controls.unsigned(b'q', 0) == 0 {
            result.replies.push(reply(
                controls,
                dispatch.image_id,
                dispatch.image_number,
                dispatch.placement_id,
                "OK",
            ));
        }
        result
    }

    fn error_result(&self, error: ProtocolError) -> KittyResult {
        let mut result = KittyResult::default();
        let id = error.controls.unsigned(b'i', 0);
        let number = error.controls.unsigned(b'I', 0);
        let addressed = id != 0 || number != 0 || error.controls.byte(b'a', b't') == b'q';
        if addressed && error.controls.unsigned(b'q', 0) != 2 {
            let placement = error.controls.unsigned(b'p', 0);
            result.replies.push(reply(
                &error.controls,
                id,
                number,
                placement,
                &format!("{}: {}", error.code, printable(&error.message)),
            ));
        }
        result
    }

    fn buffer(&self) -> &Buffer {
        if self.alternate_active {
            &self.alternate
        } else {
            &self.primary
        }
    }

    fn buffer_mut(&mut self) -> &mut Buffer {
        if self.alternate_active {
            &mut self.alternate
        } else {
            &mut self.primary
        }
    }

    fn physical_indices_in_rect(
        &self,
        grid: &Grid<Cell>,
        context: KittyContext,
        column: Option<i32>,
        row: Option<i32>,
        z_index: Option<i32>,
    ) -> Vec<(usize, usize)> {
        let buffer = self.buffer();
        let (_, virtual_anchors, _) = virtual_placements(buffer, grid, 0, context);
        let mut output = Vec::new();
        for (image_index, image) in buffer.images.iter().enumerate() {
            for (placement_index, placement) in image.placements.iter().enumerate() {
                if placement.is_virtual() || z_index.is_some_and(|z| z != placement.z_index) {
                    continue;
                }
                let mut seen = Vec::new();
                let Some(geometry) = resolve_geometry(
                    buffer,
                    image_index,
                    placement_index,
                    &virtual_anchors,
                    context,
                    &mut seen,
                    0,
                ) else {
                    continue;
                };
                let Some(placement) = clipped_output(image, placement, geometry, context) else {
                    continue;
                };
                if intersects_cell_axes(&placement, context, column, row) {
                    output.push((image_index, placement_index));
                }
            }
        }
        output
    }
}

#[derive(Debug)]
struct Dispatch {
    image_id: u32,
    image_number: u32,
    placement_id: u32,
    cursor: Option<(i32, usize)>,
    changed: bool,
}

fn build_placement(
    controls: &Controls,
    frame: &FrameImage,
    context: KittyContext,
) -> Result<StoredPlacement, ProtocolError> {
    let unicode = controls.unsigned(b'U', 0);
    if unicode > 1 || controls.unsigned(b'C', 0) > 1 {
        return Err(protocol_error(
            "EINVAL",
            "invalid Kitty placement policy",
            controls,
        ));
    }
    let source = source_rect(controls, frame)?;
    let placement_id = controls.unsigned(b'p', 0);
    let z_index = controls.signed(b'z', 0);
    if unicode != 0 {
        if controls.get(b'P').is_some() || controls.get(b'Q').is_some() {
            return Err(protocol_error(
                "EINVAL",
                "virtual placements cannot be relative",
                controls,
            ));
        }
        if controls.unsigned(b'X', 0) != 0 || controls.unsigned(b'Y', 0) != 0 {
            return Err(protocol_error(
                "EINVAL",
                "virtual placements cannot have pixel offsets",
                controls,
            ));
        }
        let columns = controls.unsigned(b'c', 0);
        let rows = controls.unsigned(b'r', 0);
        if columns == 0 || rows == 0 {
            return Err(protocol_error(
                "EINVAL",
                "virtual placements require c and r",
                controls,
            ));
        }
        return Ok(StoredPlacement {
            id: placement_id,
            source,
            z_index,
            kind: PlacementKind::Virtual { columns, rows },
        });
    }

    if context.cell_width == 0 || context.cell_height == 0 {
        return Err(protocol_error(
            "EINVAL",
            "terminal cell geometry is unavailable",
            controls,
        ));
    }
    let destination = match (controls.unsigned(b'c', 0), controls.unsigned(b'r', 0)) {
        (0, 0) => Destination::Natural,
        (columns, 0) => Destination::WidthCells(columns),
        (0, rows) => Destination::HeightCells(rows),
        (columns, rows) => Destination::Cells { columns, rows },
    };
    let (_, height, _, rows) = destination_pixels(destination, source, context);
    let offset_x = controls.unsigned(b'X', 0);
    let offset_y = controls.unsigned(b'Y', 0);
    if offset_x >= context.cell_width as u32 || offset_y >= context.cell_height as u32 {
        return Err(protocol_error(
            "EINVAL",
            "placement pixel offset exceeds its cell",
            controls,
        ));
    }
    let offset_x = offset_x as i32;
    let offset_y = offset_y as i32;
    let anchor = match (controls.get(b'P'), controls.get(b'Q')) {
        (None, None) => Anchor::Cursor {
            column: context.cursor_column.min(i32::MAX as usize) as i32,
            row: context.cursor_row,
            offset_x,
            offset_y,
        },
        (Some(parent_image), Some(parent_placement))
            if parent_image > 0 && parent_placement > 0 =>
        {
            Anchor::Relative {
                parent: ParentKey {
                    image_id: parent_image as u32,
                    placement_id: parent_placement as u32,
                },
                horizontal: controls.signed(b'H', 0),
                vertical: controls.signed(b'V', 0),
                offset_x,
                offset_y,
            }
        }
        _ => {
            return Err(protocol_error(
                "EINVAL",
                "relative placements require positive P and Q",
                controls,
            ))
        }
    };

    let row_span = rows.max(ceil_div(
        (offset_y.max(0) as u32).saturating_add(height),
        context.cell_height as u32,
    ));
    Ok(StoredPlacement {
        id: placement_id,
        source,
        z_index,
        kind: PlacementKind::Physical {
            anchor,
            destination,
            clip_rows: None,
            row_span,
        },
    })
}

fn validate_parent(
    buffer: &Buffer,
    placement: &StoredPlacement,
    target_image: u32,
    replacing_image: u32,
    controls: &Controls,
) -> Result<(), ProtocolError> {
    let Some(parent) = placement.relative_parent() else {
        return Ok(());
    };
    let own = placement.parent_key(target_image);
    if own == Some(parent) {
        return Err(protocol_error(
            "ECYCLE",
            "a placement cannot be its own parent",
            controls,
        ));
    }
    if parent.image_id == replacing_image && replacing_image != 0 {
        return Err(protocol_error(
            "ENOPARENT",
            "transmission replaces the requested parent image",
            controls,
        ));
    }
    let Some(mut current) = buffer.parent(parent) else {
        return Err(protocol_error(
            "ENOPARENT",
            "relative placement parent was not found",
            controls,
        ));
    };
    for depth in 0..=MAX_RELATIVE_DEPTH {
        let parent_placement = &buffer.images[current.0].placements[current.1];
        if parent_placement.parent_key(buffer.images[current.0].id) == own {
            return Err(protocol_error(
                "ECYCLE",
                "relative placement would create a cycle",
                controls,
            ));
        }
        let Some(next) = parent_placement.relative_parent() else {
            return Ok(());
        };
        current = buffer.parent(next).ok_or_else(|| {
            protocol_error(
                "ENOPARENT",
                "relative placement parent was removed",
                controls,
            )
        })?;
        if depth == MAX_RELATIVE_DEPTH {
            return Err(protocol_error(
                "ETOODEEP",
                "relative placement chain is too deep",
                controls,
            ));
        }
    }
    Err(protocol_error(
        "ETOODEEP",
        "relative placement chain is too deep",
        controls,
    ))
}

fn source_rect(controls: &Controls, frame: &FrameImage) -> Result<SourceRect, ProtocolError> {
    let x = controls.unsigned(b'x', 0);
    let y = controls.unsigned(b'y', 0);
    if x >= frame.width || y >= frame.height {
        return Err(protocol_error(
            "EINVAL",
            "source rectangle starts outside the image",
            controls,
        ));
    }
    let width = controls.unsigned(b'w', 0);
    let height = controls.unsigned(b'h', 0);
    let width = if width == 0 {
        frame.width - x
    } else {
        width.min(frame.width - x)
    };
    let height = if height == 0 {
        frame.height - y
    } else {
        height.min(frame.height - y)
    };
    if width == 0 || height == 0 {
        return Err(protocol_error(
            "EINVAL",
            "source rectangle is empty",
            controls,
        ));
    }
    Ok(SourceRect {
        x,
        y,
        width,
        height,
    })
}

fn destination_pixels(
    destination: Destination,
    source: SourceRect,
    context: KittyContext,
) -> (u32, u32, u32, u32) {
    let cell_width = context.cell_width.max(1) as u32;
    let cell_height = context.cell_height.max(1) as u32;
    match destination {
        Destination::Natural => {
            let columns = ceil_div(source.width, cell_width);
            let rows = ceil_div(source.height, cell_height);
            (source.width, source.height, columns, rows)
        }
        Destination::Cells { columns, rows } => (
            columns.saturating_mul(cell_width),
            rows.saturating_mul(cell_height),
            columns,
            rows,
        ),
        Destination::WidthCells(columns) => {
            let width = columns.saturating_mul(cell_width);
            let height = mul_div_ceil(source.height, width, source.width);
            (width, height, columns, ceil_div(height, cell_height))
        }
        Destination::HeightCells(rows) => {
            let height = rows.saturating_mul(cell_height);
            let width = mul_div_ceil(source.width, height, source.height);
            (width, height, ceil_div(width, cell_width), rows)
        }
    }
}

fn cursor_after(
    placement: &StoredPlacement,
    context: KittyContext,
    move_cursor: bool,
) -> Option<(i32, usize)> {
    if !move_cursor {
        return None;
    }
    let PlacementKind::Physical {
        anchor: Anchor::Cursor { column, row, .. },
        destination,
        ..
    } = placement.kind
    else {
        return None;
    };
    let (_, _, columns, rows) = destination_pixels(destination, placement.source, context);
    Some((
        row.saturating_add(rows as i32),
        column.saturating_add(columns as i32).max(0) as usize,
    ))
}

fn insert_placement(image: &mut Image, placement: StoredPlacement) {
    if placement.id != 0 {
        if let Some(index) = image
            .placements
            .iter()
            .position(|existing| existing.id == placement.id)
        {
            image.placements[index] = placement;
            return;
        }
    }
    image.placements.push(placement);
}

fn resolve_geometry(
    buffer: &Buffer,
    image_index: usize,
    placement_index: usize,
    anchors: &std::collections::HashMap<ParentKey, VirtualAnchor>,
    context: KittyContext,
    seen: &mut Vec<ParentKey>,
    depth: usize,
) -> Option<ResolvedGeometry> {
    if depth > MAX_RELATIVE_DEPTH {
        return None;
    }
    let image = buffer.images.get(image_index)?;
    let placement = image.placements.get(placement_index)?;
    let own = placement.parent_key(image.id);
    if own.is_some_and(|key| seen.contains(&key)) {
        return None;
    }
    if let Some(key) = own {
        seen.push(key);
    }

    let result = match placement.kind {
        PlacementKind::Physical {
            anchor,
            destination,
            clip_rows,
            ..
        } => {
            let (width, height, _, _) = destination_pixels(destination, placement.source, context);
            let (x, y) = match anchor {
                Anchor::Cursor {
                    column,
                    row,
                    offset_x,
                    offset_y,
                } => (
                    column
                        .saturating_mul(context.cell_width as i32)
                        .saturating_add(offset_x),
                    row.saturating_mul(context.cell_height as i32)
                        .saturating_add(offset_y),
                ),
                Anchor::Relative {
                    parent,
                    horizontal,
                    vertical,
                    offset_x,
                    offset_y,
                } => {
                    let parent_geometry = if let Some((parent_image, parent_placement)) =
                        buffer.parent(parent)
                    {
                        if buffer.images[parent_image].placements[parent_placement].is_virtual() {
                            anchors.get(&parent).map(|anchor| (anchor.x, anchor.y))
                        } else {
                            resolve_geometry(
                                buffer,
                                parent_image,
                                parent_placement,
                                anchors,
                                context,
                                seen,
                                depth + 1,
                            )
                            .map(|geometry| (geometry.x, geometry.y))
                        }
                    } else {
                        anchors.get(&parent).map(|anchor| (anchor.x, anchor.y))
                    }?;
                    (
                        parent_geometry
                            .0
                            .saturating_add(horizontal.saturating_mul(context.cell_width as i32))
                            .saturating_add(offset_x),
                        parent_geometry
                            .1
                            .saturating_add(vertical.saturating_mul(context.cell_height as i32))
                            .saturating_add(offset_y),
                    )
                }
            };
            Some(ResolvedGeometry {
                x,
                y,
                width,
                height,
                clip_rows,
            })
        }
        PlacementKind::Virtual { .. } => None,
    };

    if own.is_some() {
        seen.pop();
    }
    result
}

fn clipped_output(
    image: &Image,
    placement: &StoredPlacement,
    geometry: ResolvedGeometry,
    context: KittyContext,
) -> Option<KittyPlacement> {
    let mut clip = PixelRect {
        x: geometry.x,
        y: geometry.y,
        width: geometry.width,
        height: geometry.height,
    };
    clip = intersect_pixel_rect(
        clip,
        PixelRect {
            x: 0,
            y: 0,
            width: (context.columns as u32).saturating_mul(context.cell_width as u32),
            height: (context.lines as u32).saturating_mul(context.cell_height as u32),
        },
    )?;
    if let Some((low, high)) = geometry.clip_rows {
        clip = intersect_pixel_rect(
            clip,
            PixelRect {
                x: i32::MIN / 2,
                y: low.saturating_mul(context.cell_height as i32),
                width: u32::MAX,
                height: high.saturating_sub(low).max(0) as u32 * context.cell_height as u32,
            },
        )?;
    }
    Some(KittyPlacement {
        image_id: image.frames.current().id,
        x: geometry.x,
        y: geometry.y,
        width: geometry.width,
        height: geometry.height,
        source_x: placement.source.x,
        source_y: placement.source.y,
        source_width: placement.source.width,
        source_height: placement.source.height,
        clip_x: clip.x,
        clip_y: clip.y,
        clip_width: clip.width,
        clip_height: clip.height,
        z_index: placement.z_index,
        order: image.id,
    })
}

#[derive(Clone, Copy)]
struct PixelRect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

fn intersect_pixel_rect(left: PixelRect, right: PixelRect) -> Option<PixelRect> {
    let left_right = (left.x as i64).saturating_add(left.width as i64);
    let left_bottom = (left.y as i64).saturating_add(left.height as i64);
    let right_right = (right.x as i64).saturating_add(right.width as i64);
    let right_bottom = (right.y as i64).saturating_add(right.height as i64);
    let x = (left.x as i64).max(right.x as i64);
    let y = (left.y as i64).max(right.y as i64);
    let width = left_right.min(right_right).saturating_sub(x);
    let height = left_bottom.min(right_bottom).saturating_sub(y);
    (width > 0 && height > 0).then_some(PixelRect {
        x: x as i32,
        y: y as i32,
        width: width as u32,
        height: height as u32,
    })
}

fn intersects_cell_axes(
    placement: &KittyPlacement,
    context: KittyContext,
    column: Option<i32>,
    row: Option<i32>,
) -> bool {
    let intersects_axis = |origin: i32, length: u32, cell: i32, extent: u16| {
        let start = (cell as i64 - 1).saturating_mul(extent as i64);
        let end = start.saturating_add(extent as i64);
        let origin = origin as i64;
        let far_edge = origin.saturating_add(length as i64);
        far_edge > start && origin < end
    };
    column.is_none_or(|column| {
        intersects_axis(
            placement.clip_x,
            placement.clip_width,
            column,
            context.cell_width,
        )
    }) && row.is_none_or(|row| {
        intersects_axis(
            placement.clip_y,
            placement.clip_height,
            row,
            context.cell_height,
        )
    })
}

fn virtual_placements(
    buffer: &Buffer,
    grid: &Grid<Cell>,
    display_offset: usize,
    context: KittyContext,
) -> (
    Vec<KittyPlacement>,
    std::collections::HashMap<ParentKey, VirtualAnchor>,
    Vec<u64>,
) {
    let mut placements = Vec::new();
    let mut anchors = std::collections::HashMap::new();
    let mut image_ids = Vec::new();

    let display_offset = display_offset.min(i32::MAX as usize) as i32;
    let history = context.history_size.min(i32::MAX as usize) as i32;
    for grid_row in -history..context.lines.min(i32::MAX as usize) as i32 {
        let display_row = grid_row.saturating_add(display_offset);
        let visible = (0..context.lines.min(i32::MAX as usize) as i32).contains(&display_row);
        let row = &grid[Line(grid_row)];
        let mut previous = None;
        for column in 0..context.columns.min(row.len()) {
            let cell = &row[Column(column)];
            let Some(code) = decode_placeholder(cell, previous.as_ref()) else {
                previous = None;
                continue;
            };
            previous = Some(code.clone());
            let Some((image_index, placement_index)) =
                buffer.virtual_placement(code.image_id, code.placement_id)
            else {
                continue;
            };
            let image = &buffer.images[image_index];
            let placement = &image.placements[placement_index];
            let PlacementKind::Virtual { columns, rows } = placement.kind else {
                continue;
            };
            if code.row >= rows as usize || code.column >= columns as usize {
                continue;
            }
            let key = placement.parent_key(image.id);
            if let Some(key) = key {
                let x = (column as u32)
                    .saturating_mul(context.cell_width as u32)
                    .min(i32::MAX as u32) as i32;
                let y = grid_row.saturating_mul(context.cell_height as i32);
                anchors
                    .entry(key)
                    .and_modify(|anchor: &mut VirtualAnchor| {
                        anchor.x = anchor.x.min(x);
                        anchor.y = anchor.y.min(y);
                    })
                    .or_insert(VirtualAnchor { x, y });
            }
            if !visible {
                continue;
            }
            let Some(output) = virtual_output(
                image,
                placement,
                columns,
                rows,
                code.row,
                code.column,
                display_row as usize,
                column,
                context,
            ) else {
                continue;
            };
            append_image_resources(&mut image_ids, &image.frames);
            placements.push(output);
        }
    }

    (placements, anchors, image_ids)
}

fn virtual_output(
    image: &Image,
    placement: &StoredPlacement,
    columns: u32,
    rows: u32,
    source_row: usize,
    source_column: usize,
    display_row: usize,
    display_column: usize,
    context: KittyContext,
) -> Option<KittyPlacement> {
    let cell_width = context.cell_width as u32;
    let cell_height = context.cell_height as u32;
    if cell_width == 0 || cell_height == 0 {
        return None;
    }
    let available_width = columns.checked_mul(cell_width)?;
    let available_height = rows.checked_mul(cell_height)?;
    let (width, height) = fit_size(
        placement.source.width,
        placement.source.height,
        available_width,
        available_height,
    )?;
    let padding_x = (available_width - width) / 2;
    let padding_y = (available_height - height) / 2;
    let origin_x = (display_column as i64)
        .saturating_mul(cell_width as i64)
        .saturating_sub((source_column as i64).saturating_mul(cell_width as i64))
        .saturating_add(padding_x as i64)
        .clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    let origin_y = (display_row as i64)
        .saturating_mul(cell_height as i64)
        .saturating_sub((source_row as i64).saturating_mul(cell_height as i64))
        .saturating_add(padding_y as i64)
        .clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    let clip = intersect_pixel_rect(
        PixelRect {
            x: (display_column as u32)
                .saturating_mul(cell_width)
                .min(i32::MAX as u32) as i32,
            y: (display_row as u32)
                .saturating_mul(cell_height)
                .min(i32::MAX as u32) as i32,
            width: cell_width,
            height: cell_height,
        },
        PixelRect {
            x: origin_x,
            y: origin_y,
            width,
            height,
        },
    )?;
    Some(KittyPlacement {
        image_id: image.frames.current().id,
        x: origin_x,
        y: origin_y,
        width,
        height,
        source_x: placement.source.x,
        source_y: placement.source.y,
        source_width: placement.source.width,
        source_height: placement.source.height,
        clip_x: clip.x,
        clip_y: clip.y,
        clip_width: clip.width,
        clip_height: clip.height,
        z_index: placement.z_index,
        order: image.id,
    })
}

#[derive(Clone)]
struct PlaceholderCode {
    foreground: Color,
    underline: Option<Color>,
    image_id: u32,
    placement_id: u32,
    row: usize,
    column: usize,
    high_byte: u8,
}

fn decode_placeholder(cell: &Cell, previous: Option<&PlaceholderCode>) -> Option<PlaceholderCode> {
    if cell.c != PLACEHOLDER {
        return None;
    }
    let foreground = cell.fg;
    let underline = cell.underline_color();
    let low = color_id(foreground)?;
    let placement_id = underline.and_then(color_id).unwrap_or(0);
    let mut diacritics = cell
        .zerowidth()
        .into_iter()
        .flatten()
        .filter_map(|character| ROWCOLUMN_DIACRITICS.binary_search(character).ok());
    let row_diacritic = diacritics.next();
    let column_diacritic = diacritics.next();
    let high_diacritic = diacritics.next();
    let inherited = previous
        .filter(|previous| previous.foreground == foreground && previous.underline == underline);
    let (row, column, high_byte) = match (row_diacritic, column_diacritic, high_diacritic) {
        (None, None, None) => {
            let previous = inherited?;
            (
                previous.row,
                previous.column.checked_add(1)?,
                previous.high_byte,
            )
        }
        (Some(row), None, None) => match inherited.filter(|previous| previous.row == row) {
            Some(previous) => (row, previous.column.checked_add(1)?, previous.high_byte),
            None => (row, 0, 0),
        },
        (Some(row), Some(column), None) => {
            let high_byte = inherited
                .filter(|previous| {
                    previous.row == row && previous.column.checked_add(1) == Some(column)
                })
                .map_or(0, |previous| previous.high_byte);
            (row, column, high_byte)
        }
        (Some(row), Some(column), Some(high)) if high <= u8::MAX as usize => {
            (row, column, high as u8)
        }
        _ => return None,
    };
    let image_id = low | ((high_byte as u32) << 24);
    Some(PlaceholderCode {
        foreground,
        underline,
        image_id,
        placement_id,
        row,
        column,
        high_byte,
    })
}

fn color_id(color: Color) -> Option<u32> {
    match color {
        Color::Indexed(index) => Some(index as u32),
        Color::Spec(rgb) => Some(((rgb.r as u32) << 16) | ((rgb.g as u32) << 8) | rgb.b as u32),
        Color::Named(_) => None,
    }
}

fn take_image(buffer: &mut Buffer, index: usize) -> (Vec<u64>, Vec<ParentKey>) {
    let image = buffer.images.remove(index);
    let parent_keys = image
        .placements
        .iter()
        .filter_map(|placement| placement.parent_key(image.id))
        .collect();
    let frame_ids = image.frames.images().map(|frame| frame.id).collect();
    (frame_ids, parent_keys)
}

fn append_image_resources(output: &mut Vec<u64>, frames: &Frames) {
    output.extend(frames.images().map(|frame| frame.id));
}

fn frame_images(frames: &Frames) -> Vec<FrameImage> {
    frames.images().cloned().collect()
}

fn remove_all_images(buffer: &mut Buffer) -> Vec<u64> {
    let mut ids = Vec::new();
    for image in buffer.images.drain(..) {
        ids.extend(image.frames.images().map(|frame| frame.id));
    }
    ids
}

fn required_cell_coordinate(controls: &Controls, key: u8) -> Result<i32, ProtocolError> {
    let coordinate = controls.unsigned(key, 0);
    if coordinate == 0 || coordinate > i32::MAX as u32 {
        return Err(protocol_error(
            "EINVAL",
            format!("{} must name a positive cell", key as char),
            controls,
        ));
    }
    Ok(coordinate as i32)
}

fn intersect_rows(existing: Option<(i32, i32)>, next: (i32, i32)) -> Option<(i32, i32)> {
    Some(match existing {
        Some((low, high)) => (low.max(next.0), high.min(next.1)),
        None => next,
    })
}

fn ceil_div(value: u32, divisor: u32) -> u32 {
    value / divisor + u32::from(value % divisor != 0)
}

fn mul_div_ceil(left: u32, right: u32, divisor: u32) -> u32 {
    (((left as u64).saturating_mul(right as u64) + divisor as u64 - 1) / divisor as u64)
        .min(u32::MAX as u64) as u32
}

fn fit_size(
    source_width: u32,
    source_height: u32,
    max_width: u32,
    max_height: u32,
) -> Option<(u32, u32)> {
    if source_width == 0 || source_height == 0 || max_width == 0 || max_height == 0 {
        return None;
    }
    if (source_width as u64).saturating_mul(max_height as u64)
        <= (source_height as u64).saturating_mul(max_width as u64)
    {
        Some((
            mul_div_ceil(source_width, max_height, source_height),
            max_height,
        ))
    } else {
        Some((
            max_width,
            mul_div_ceil(source_height, max_width, source_width),
        ))
    }
}

fn protocol_error(
    code: &'static str,
    message: impl Into<String>,
    controls: &Controls,
) -> ProtocolError {
    ProtocolError::new(code, message, controls.clone())
}

fn reply(
    _controls: &Controls,
    image_id: u32,
    image_number: u32,
    placement_id: u32,
    status: &str,
) -> String {
    let mut fields = Vec::new();
    if image_id != 0 {
        fields.push(format!("i={image_id}"));
    }
    if image_number != 0 {
        fields.push(format!("I={image_number}"));
    }
    if placement_id != 0 {
        fields.push(format!("p={placement_id}"));
    }
    format!("\x1b_G{};{}\x1b\\", fields.join(","), status)
}

fn printable(message: &str) -> String {
    message
        .chars()
        .map(|character| {
            if character.is_ascii_graphic() || character == ' ' {
                character
            } else {
                ' '
            }
        })
        .collect()
}

// kitty's `gen/rowcolumn-diacritics.txt`, kept as scalars so lookup preserves
// the protocol's exact 297-entry index table.
const ROWCOLUMN_DIACRITICS: [char; 297] = [
    '\u{305}',
    '\u{30d}',
    '\u{30e}',
    '\u{310}',
    '\u{312}',
    '\u{33d}',
    '\u{33e}',
    '\u{33f}',
    '\u{346}',
    '\u{34a}',
    '\u{34b}',
    '\u{34c}',
    '\u{350}',
    '\u{351}',
    '\u{352}',
    '\u{357}',
    '\u{35b}',
    '\u{363}',
    '\u{364}',
    '\u{365}',
    '\u{366}',
    '\u{367}',
    '\u{368}',
    '\u{369}',
    '\u{36a}',
    '\u{36b}',
    '\u{36c}',
    '\u{36d}',
    '\u{36e}',
    '\u{36f}',
    '\u{483}',
    '\u{484}',
    '\u{485}',
    '\u{486}',
    '\u{487}',
    '\u{592}',
    '\u{593}',
    '\u{594}',
    '\u{595}',
    '\u{597}',
    '\u{598}',
    '\u{599}',
    '\u{59c}',
    '\u{59d}',
    '\u{59e}',
    '\u{59f}',
    '\u{5a0}',
    '\u{5a1}',
    '\u{5a8}',
    '\u{5a9}',
    '\u{5ab}',
    '\u{5ac}',
    '\u{5af}',
    '\u{5c4}',
    '\u{610}',
    '\u{611}',
    '\u{612}',
    '\u{613}',
    '\u{614}',
    '\u{615}',
    '\u{616}',
    '\u{617}',
    '\u{657}',
    '\u{658}',
    '\u{659}',
    '\u{65a}',
    '\u{65b}',
    '\u{65d}',
    '\u{65e}',
    '\u{6d6}',
    '\u{6d7}',
    '\u{6d8}',
    '\u{6d9}',
    '\u{6da}',
    '\u{6db}',
    '\u{6dc}',
    '\u{6df}',
    '\u{6e0}',
    '\u{6e1}',
    '\u{6e2}',
    '\u{6e4}',
    '\u{6e7}',
    '\u{6e8}',
    '\u{6eb}',
    '\u{6ec}',
    '\u{730}',
    '\u{732}',
    '\u{733}',
    '\u{735}',
    '\u{736}',
    '\u{73a}',
    '\u{73d}',
    '\u{73f}',
    '\u{740}',
    '\u{741}',
    '\u{743}',
    '\u{745}',
    '\u{747}',
    '\u{749}',
    '\u{74a}',
    '\u{7eb}',
    '\u{7ec}',
    '\u{7ed}',
    '\u{7ee}',
    '\u{7ef}',
    '\u{7f0}',
    '\u{7f1}',
    '\u{7f3}',
    '\u{816}',
    '\u{817}',
    '\u{818}',
    '\u{819}',
    '\u{81b}',
    '\u{81c}',
    '\u{81d}',
    '\u{81e}',
    '\u{81f}',
    '\u{820}',
    '\u{821}',
    '\u{822}',
    '\u{823}',
    '\u{825}',
    '\u{826}',
    '\u{827}',
    '\u{829}',
    '\u{82a}',
    '\u{82b}',
    '\u{82c}',
    '\u{82d}',
    '\u{951}',
    '\u{953}',
    '\u{954}',
    '\u{f82}',
    '\u{f83}',
    '\u{f86}',
    '\u{f87}',
    '\u{135d}',
    '\u{135e}',
    '\u{135f}',
    '\u{17dd}',
    '\u{193a}',
    '\u{1a17}',
    '\u{1a75}',
    '\u{1a76}',
    '\u{1a77}',
    '\u{1a78}',
    '\u{1a79}',
    '\u{1a7a}',
    '\u{1a7b}',
    '\u{1a7c}',
    '\u{1b6b}',
    '\u{1b6d}',
    '\u{1b6e}',
    '\u{1b6f}',
    '\u{1b70}',
    '\u{1b71}',
    '\u{1b72}',
    '\u{1b73}',
    '\u{1cd0}',
    '\u{1cd1}',
    '\u{1cd2}',
    '\u{1cda}',
    '\u{1cdb}',
    '\u{1ce0}',
    '\u{1dc0}',
    '\u{1dc1}',
    '\u{1dc3}',
    '\u{1dc4}',
    '\u{1dc5}',
    '\u{1dc6}',
    '\u{1dc7}',
    '\u{1dc8}',
    '\u{1dc9}',
    '\u{1dcb}',
    '\u{1dcc}',
    '\u{1dd1}',
    '\u{1dd2}',
    '\u{1dd3}',
    '\u{1dd4}',
    '\u{1dd5}',
    '\u{1dd6}',
    '\u{1dd7}',
    '\u{1dd8}',
    '\u{1dd9}',
    '\u{1dda}',
    '\u{1ddb}',
    '\u{1ddc}',
    '\u{1ddd}',
    '\u{1dde}',
    '\u{1ddf}',
    '\u{1de0}',
    '\u{1de1}',
    '\u{1de2}',
    '\u{1de3}',
    '\u{1de4}',
    '\u{1de5}',
    '\u{1de6}',
    '\u{1dfe}',
    '\u{20d0}',
    '\u{20d1}',
    '\u{20d4}',
    '\u{20d5}',
    '\u{20d6}',
    '\u{20d7}',
    '\u{20db}',
    '\u{20dc}',
    '\u{20e1}',
    '\u{20e7}',
    '\u{20e9}',
    '\u{20f0}',
    '\u{2cef}',
    '\u{2cf0}',
    '\u{2cf1}',
    '\u{2de0}',
    '\u{2de1}',
    '\u{2de2}',
    '\u{2de3}',
    '\u{2de4}',
    '\u{2de5}',
    '\u{2de6}',
    '\u{2de7}',
    '\u{2de8}',
    '\u{2de9}',
    '\u{2dea}',
    '\u{2deb}',
    '\u{2dec}',
    '\u{2ded}',
    '\u{2dee}',
    '\u{2def}',
    '\u{2df0}',
    '\u{2df1}',
    '\u{2df2}',
    '\u{2df3}',
    '\u{2df4}',
    '\u{2df5}',
    '\u{2df6}',
    '\u{2df7}',
    '\u{2df8}',
    '\u{2df9}',
    '\u{2dfa}',
    '\u{2dfb}',
    '\u{2dfc}',
    '\u{2dfd}',
    '\u{2dfe}',
    '\u{2dff}',
    '\u{a66f}',
    '\u{a67c}',
    '\u{a67d}',
    '\u{a6f0}',
    '\u{a6f1}',
    '\u{a8e0}',
    '\u{a8e1}',
    '\u{a8e2}',
    '\u{a8e3}',
    '\u{a8e4}',
    '\u{a8e5}',
    '\u{a8e6}',
    '\u{a8e7}',
    '\u{a8e8}',
    '\u{a8e9}',
    '\u{a8ea}',
    '\u{a8eb}',
    '\u{a8ec}',
    '\u{a8ed}',
    '\u{a8ee}',
    '\u{a8ef}',
    '\u{a8f0}',
    '\u{a8f1}',
    '\u{aab0}',
    '\u{aab2}',
    '\u{aab3}',
    '\u{aab7}',
    '\u{aab8}',
    '\u{aabe}',
    '\u{aabf}',
    '\u{aac1}',
    '\u{fe20}',
    '\u{fe21}',
    '\u{fe22}',
    '\u{fe23}',
    '\u{fe24}',
    '\u{fe25}',
    '\u{fe26}',
    '\u{10a0f}',
    '\u{10a38}',
    '\u{1d185}',
    '\u{1d186}',
    '\u{1d187}',
    '\u{1d188}',
    '\u{1d189}',
    '\u{1d1aa}',
    '\u{1d1ab}',
    '\u{1d1ac}',
    '\u{1d1ad}',
    '\u{1d242}',
    '\u{1d243}',
    '\u{1d244}',
];

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::grid::Grid;
    use crate::index::{Column, Line};
    use crate::term::cell::Cell;
    use crate::vte::ansi::Color;

    use super::{
        Anchor, Destination, FrameImage, Frames, Image, KittyContext, KittyState, PlacementKind,
        SourceRect, StoredPlacement, PLACEHOLDER, ROWCOLUMN_DIACRITICS,
    };

    fn context() -> KittyContext {
        KittyContext {
            cursor_row: 0,
            cursor_column: 0,
            columns: 4,
            lines: 4,
            history_size: 0,
            cell_width: 10,
            cell_height: 20,
        }
    }

    fn frame(id: u64) -> FrameImage {
        FrameImage {
            id,
            width: 2,
            height: 2,
            rgba: Arc::new(vec![0; 16]),
        }
    }

    #[test]
    fn resources_use_a_separate_monotonic_high_bit_namespace() {
        let mut state = KittyState::default();
        let first = state.next_resource_id(&Default::default()).unwrap();
        state.reset();
        let second = state.next_resource_id(&Default::default()).unwrap();

        assert_ne!(first, second);
        assert_ne!(first & super::RESOURCE_NAMESPACE, 0);
        assert_ne!(second & super::RESOURCE_NAMESPACE, 0);
        assert!(second > first);
    }

    #[test]
    fn placeholders_inherit_an_omitted_column_from_the_left() {
        let mut first_cell = Cell::default();
        first_cell.c = PLACEHOLDER;
        first_cell.fg = Color::Indexed(42);
        first_cell.push_zerowidth(ROWCOLUMN_DIACRITICS[0]);
        first_cell.push_zerowidth(ROWCOLUMN_DIACRITICS[0]);
        let first = super::decode_placeholder(&first_cell, None).unwrap();

        let mut second_cell = Cell::default();
        second_cell.c = PLACEHOLDER;
        second_cell.fg = Color::Indexed(42);
        let second = super::decode_placeholder(&second_cell, Some(&first)).unwrap();

        assert_eq!((first.row, first.column, first.image_id), (0, 0, 42));
        assert_eq!((second.row, second.column, second.image_id), (0, 1, 42));
    }

    #[test]
    fn virtual_placeholders_emit_current_frame_and_every_live_frame_resource() {
        let mut state = KittyState::default();
        state.primary.images.push(Image {
            id: 42,
            number: 0,
            transient: false,
            frames: Frames::new(frame(super::RESOURCE_NAMESPACE | 7)),
            placements: vec![StoredPlacement {
                id: 0,
                source: SourceRect {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 2,
                },
                z_index: 0,
                kind: PlacementKind::Virtual {
                    columns: 1,
                    rows: 1,
                },
            }],
        });

        let mut grid = Grid::<Cell>::new(4, 4, 0);
        let cell = &mut grid[Line(0)][Column(0)];
        cell.c = PLACEHOLDER;
        cell.fg = Color::Indexed(42);
        cell.push_zerowidth(ROWCOLUMN_DIACRITICS[0]);
        cell.push_zerowidth(ROWCOLUMN_DIACRITICS[0]);

        let scene = state.scene(&grid, 0, context());
        assert_eq!(scene.placements.len(), 1);
        assert_eq!(scene.placements[0].image_id, super::RESOURCE_NAMESPACE | 7);
        assert_eq!(scene.image_ids, vec![super::RESOURCE_NAMESPACE | 7]);
    }

    #[test]
    fn relative_placements_resolve_from_their_parent_anchor() {
        let mut state = KittyState::default();
        state.primary.images.push(Image {
            id: 7,
            number: 0,
            transient: false,
            frames: Frames::new(frame(super::RESOURCE_NAMESPACE | 9)),
            placements: vec![
                StoredPlacement {
                    id: 1,
                    source: SourceRect {
                        x: 0,
                        y: 0,
                        width: 2,
                        height: 2,
                    },
                    z_index: 0,
                    kind: PlacementKind::Physical {
                        anchor: Anchor::Cursor {
                            column: 0,
                            row: 0,
                            offset_x: 0,
                            offset_y: 0,
                        },
                        destination: Destination::Cells {
                            columns: 1,
                            rows: 1,
                        },
                        clip_rows: None,
                        row_span: 1,
                    },
                },
                StoredPlacement {
                    id: 2,
                    source: SourceRect {
                        x: 0,
                        y: 0,
                        width: 2,
                        height: 2,
                    },
                    z_index: 0,
                    kind: PlacementKind::Physical {
                        anchor: Anchor::Relative {
                            parent: super::ParentKey {
                                image_id: 7,
                                placement_id: 1,
                            },
                            horizontal: 1,
                            vertical: 1,
                            offset_x: 0,
                            offset_y: 0,
                        },
                        destination: Destination::Natural,
                        clip_rows: None,
                        row_span: 1,
                    },
                },
            ],
        });
        let grid = Grid::<Cell>::new(4, 4, 0);

        let scene = state.scene(&grid, 0, context());
        assert_eq!((scene.placements[0].x, scene.placements[0].y), (0, 0));
        assert_eq!((scene.placements[1].x, scene.placements[1].y), (10, 20));
    }
}
