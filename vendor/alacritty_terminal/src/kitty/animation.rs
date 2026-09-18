//! Immutable, fully-composited storage for Kitty animation frames.
//!
//! Frames are materialized when they are received or edited.  Playback only
//! switches `FrameImage` references, so advancing an animation never copies
//! pixel data.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::decode::{Controls, DecodedImage, ProtocolError};

const DEFAULT_GAP: Duration = Duration::from_millis(40);
const MAX_FRAMES: usize = 128;

/// Pixels and render-resource identity for one fully composited animation frame.
///
/// `id` is allocated by native Kitty state and is deliberately unrelated to a
/// client supplied Kitty image id.
#[derive(Clone, Debug)]
pub struct FrameImage {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Playback {
    Stopped,
    Loading,
    Running,
}

#[derive(Debug)]
struct Frame {
    image: FrameImage,
    /// Zero denotes a gapless frame.  The root frame begins gapless too, until
    /// an animation-control command gives it a gap.
    gap: Duration,
}

/// The frame timeline for one Kitty image.
///
/// Every retained frame is a standalone RGBA canvas.  This intentionally
/// trades frame storage for immutable render resources and cheap playback.
#[derive(Debug)]
pub struct Frames {
    frames: Vec<Frame>,
    current: usize,
    playback: Playback,
    shown_at: Instant,
    /// `None` is infinite.  Otherwise this is the number of completed wraps
    /// that may occur before playback stops on the last frame.
    maximum_loops: Option<u32>,
    completed_loops: u32,
}

impl Frames {
    pub fn new(image: FrameImage) -> Self {
        Self {
            frames: vec![Frame {
                image,
                gap: Duration::ZERO,
            }],
            current: 0,
            playback: Playback::Stopped,
            shown_at: Instant::now(),
            maximum_loops: None,
            completed_loops: 0,
        }
    }

    #[inline]
    pub fn current(&self) -> &FrameImage {
        &self.frames[self.current].image
    }

    #[inline]
    pub fn images(&self) -> impl Iterator<Item = &FrameImage> {
        self.frames.iter().map(|frame| &frame.image)
    }

    #[inline]
    pub fn bytes(&self) -> usize {
        self.frames.iter().map(|frame| frame.image.rgba.len()).sum()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Add a frame or atomically replace one frame's fully-composited canvas.
    pub fn transmit(
        &mut self,
        controls: &Controls,
        image: DecodedImage,
        resource_id: u64,
        now: Instant,
        byte_limit: usize,
    ) -> Result<(), ProtocolError> {
        let (width, height, canvas_len) = self.canvas_geometry(controls)?;
        Self::validate_image(&image, controls)?;

        let x = controls.unsigned(b'x', 0);
        let y = controls.unsigned(b'y', 0);
        Self::validate_rectangle(
            width,
            height,
            x,
            y,
            image.width,
            image.height,
            controls,
            "frame",
        )?;

        // Kitty treats omitted/out-of-range r as an append request.  Existing
        // frame numbers are one based; r=len+1 is the normal append spelling.
        let requested = controls.unsigned(b'r', 0) as usize;
        let target = if requested != 0 && requested <= self.frames.len() {
            requested - 1
        } else {
            self.frames.len()
        };
        let is_new = target == self.frames.len();

        if is_new && self.frames.len() >= MAX_FRAMES {
            return Err(Self::error(
                controls,
                "ENOSPC",
                "image already has the maximum number of frames",
            ));
        }

        let retained_before = self.bytes();
        let retained_after = if is_new {
            retained_before.checked_add(canvas_len)
        } else {
            retained_before
                .checked_sub(self.frames[target].image.rgba.len())
                .and_then(|bytes| bytes.checked_add(canvas_len))
        };
        if retained_after.is_none_or(|bytes| bytes > byte_limit) {
            return Err(Self::error(
                controls,
                "ENOSPC",
                "animation frame storage limit exceeded",
            ));
        }

        // A base frame is meaningful only when creating a new frame.  Editing
        // always starts from the existing fully composed destination.  Validate
        // it even when a full replacement makes its pixels unnecessary.
        let base_number = controls.unsigned(b'c', 0);
        if is_new && base_number != 0 {
            self.frame_index(base_number, controls, "base")?;
        }
        let background = controls.unsigned(b'Y', 0);
        let replacement = controls.unsigned(b'X', 0) == 1;
        // A full replacement has no dependency on the old canvas, so retain
        // the decoder's immutable Arc rather than copying it into one.
        let can_adopt_input =
            replacement && x == 0 && y == 0 && image.width == width && image.height == height;
        let rgba = if can_adopt_input {
            image.rgba
        } else {
            let mut pixels = if is_new {
                if base_number == 0 {
                    Self::background(canvas_len, background)
                } else {
                    let base = self.frame_index(base_number, controls, "base")?;
                    self.frames[base].image.rgba.as_ref().clone()
                }
            } else {
                self.frames[target].image.rgba.as_ref().clone()
            };
            Self::overlay(
                &mut pixels,
                width,
                x,
                y,
                image.rgba.as_ref(),
                image.width,
                image.height,
                replacement,
            );
            Arc::new(pixels)
        };

        let gap = if is_new {
            Self::new_frame_gap(controls)
        } else {
            match controls.get(b'z') {
                Some(value) if value != 0 => Self::gap(value),
                _ => self.frames[target].gap,
            }
        };
        let frame = Frame {
            image: FrameImage {
                id: resource_id,
                width,
                height,
                rgba,
            },
            gap,
        };

        if is_new {
            self.frames.push(frame);
            // In loading mode, an expired last frame must advance as soon as a
            // newly received successor makes that possible.  This also starts
            // a running animation which previously had only its root frame.
            if self.playback != Playback::Stopped {
                self.tick(now);
            }
        } else {
            self.frames[target] = frame;
        }

        Ok(())
    }

    /// Compose a source frame rectangle onto a destination frame atomically.
    pub fn compose(
        &mut self,
        controls: &Controls,
        resource_id: u64,
        _now: Instant,
        byte_limit: usize,
    ) -> Result<(), ProtocolError> {
        let (width, height, canvas_len) = self.canvas_geometry(controls)?;

        // Kitty's implementation resolves r as the source and c as the
        // destination, despite older table prose that described them inversely.
        let source = self.frame_index(controls.unsigned(b'r', 0), controls, "source")?;
        let destination = self.frame_index(controls.unsigned(b'c', 0), controls, "destination")?;

        let rect_width = match controls.get(b'w') {
            Some(0) | None => width,
            Some(value) => value as u32,
        };
        let rect_height = match controls.get(b'h') {
            Some(0) | None => height,
            Some(value) => value as u32,
        };
        let destination_x = controls.unsigned(b'x', 0);
        let destination_y = controls.unsigned(b'y', 0);
        let source_x = controls.unsigned(b'X', 0);
        let source_y = controls.unsigned(b'Y', 0);

        Self::validate_rectangle(
            width,
            height,
            source_x,
            source_y,
            rect_width,
            rect_height,
            controls,
            "source",
        )?;
        Self::validate_rectangle(
            width,
            height,
            destination_x,
            destination_y,
            rect_width,
            rect_height,
            controls,
            "destination",
        )?;
        if source == destination
            && Self::rectangles_overlap(
                source_x,
                source_y,
                destination_x,
                destination_y,
                rect_width,
                rect_height,
            )
        {
            return Err(Self::error(
                controls,
                "EINVAL",
                "source and destination rectangles overlap in the same frame",
            ));
        }

        let retained_after = self
            .bytes()
            .checked_sub(self.frames[destination].image.rgba.len())
            .and_then(|bytes| bytes.checked_add(canvas_len));
        if retained_after.is_none_or(|bytes| bytes > byte_limit) {
            return Err(Self::error(
                controls,
                "ENOSPC",
                "animation frame storage limit exceeded",
            ));
        }

        let replacement = controls.unsigned(b'C', 0) == 1;
        let can_adopt_source = replacement
            && source_x == 0
            && source_y == 0
            && destination_x == 0
            && destination_y == 0
            && rect_width == width
            && rect_height == height;
        let rgba = if can_adopt_source {
            self.frames[source].image.rgba.clone()
        } else {
            let mut pixels = self.frames[destination].image.rgba.as_ref().clone();
            let source_pixels = self.frames[source].image.rgba.as_ref();
            Self::overlay_region(
                &mut pixels,
                width,
                destination_x,
                destination_y,
                source_pixels,
                width,
                source_x,
                source_y,
                rect_width,
                rect_height,
                replacement,
            );
            Arc::new(pixels)
        };

        let gap = self.frames[destination].gap;
        self.frames[destination] = Frame {
            image: FrameImage {
                id: resource_id,
                width,
                height,
                rgba,
            },
            gap,
        };
        Ok(())
    }

    /// Apply frame timing, selection and playback controls atomically.
    pub fn control(&mut self, controls: &Controls, now: Instant) -> Result<(), ProtocolError> {
        let gap_target = match controls.get(b'r') {
            None | Some(0) => None,
            Some(value) => Some(self.frame_index(value as u32, controls, "frame")?),
        };
        if controls.get(b'z').is_some() && gap_target.is_none() {
            return Err(Self::error(
                controls,
                "EINVAL",
                "a frame number is required to change a frame gap",
            ));
        }

        let selection = match controls.get(b'c') {
            None => None,
            Some(0) => {
                return Err(Self::error(
                    controls,
                    "EINVAL",
                    "frame number must be positive",
                ))
            }
            Some(value) => Some(self.frame_index(value as u32, controls, "current")?),
        };
        let requested_state = match controls.get(b's') {
            None | Some(0) => None,
            Some(1) => Some(Playback::Stopped),
            Some(2) => Some(Playback::Loading),
            Some(3) => Some(Playback::Running),
            Some(_) => return Err(Self::error(controls, "EINVAL", "invalid animation state")),
        };

        if let Some(frame) = gap_target {
            if let Some(value) = controls.get(b'z') {
                // z=0 means unspecified and leaves an existing gap unchanged.
                if value != 0 {
                    self.frames[frame].gap = Self::gap(value);
                }
            }
        }
        if let Some(frame) = selection {
            if frame != self.current {
                self.current = frame;
                self.shown_at = now;
            }
        }
        if let Some(state) = requested_state {
            let old_state = self.playback;
            self.playback = state;
            self.completed_loops = 0;
            if state == Playback::Stopped {
                // Stopping explicitly resets loop accounting; current frame is
                // preserved for client-driven selection.
                self.completed_loops = 0;
            } else if old_state == Playback::Stopped {
                self.shown_at = now;
            }
        }
        if let Some(value) = controls.get(b'v') {
            if value != 0 {
                self.maximum_loops = if value == 1 {
                    None
                } else {
                    Some(value as u32 - 1)
                };
            }
        }

        Ok(())
    }

    /// Delete an animation frame.  Uppercase `F` may remove the final root;
    /// the caller then removes the owning image when `len()` reaches zero.
    pub fn delete(&mut self, controls: &Controls, now: Instant) -> Result<(), ProtocolError> {
        match controls.byte(b'd', b'a') {
            b'f' | b'F' => {}
            _ => {
                return Err(Self::error(
                    controls,
                    "EINVAL",
                    "not an animation frame deletion",
                ))
            }
        }
        if self.frames.is_empty() {
            return Ok(());
        }

        // Reference kitty clamps a requested frame beyond the tail to the tail
        // and defaults an omitted r to the root.
        let requested = controls.unsigned(b'r', 0) as usize;
        let target = if requested == 0 {
            0
        } else {
            requested.min(self.frames.len()) - 1
        };
        if self.frames.len() == 1 {
            if controls.byte(b'd', b'a') == b'F' {
                self.frames.clear();
                self.current = 0;
                self.playback = Playback::Stopped;
                self.completed_loops = 0;
            }
            return Ok(());
        }

        let old_current = self.current;
        self.frames.remove(target);
        if old_current == target {
            self.current = target.min(self.frames.len() - 1);
            self.shown_at = now;
        } else if old_current > target {
            self.current = old_current - 1;
        }
        Ok(())
    }

    /// Advance the terminal-driven timeline and report whether the visible
    /// frame resource differs after the catch-up.
    pub fn tick(&mut self, now: Instant) -> bool {
        if self.playback == Playback::Stopped || self.frames.len() < 2 || !self.has_timed_frame() {
            return false;
        }

        let previous_id = self.current().id;
        // At most one partial pass before and after an optional whole-cycle
        // skip is needed.  The skip makes arbitrarily stale timers cheap.
        let transition_limit = self.frames.len().saturating_mul(2).saturating_add(1);
        let mut transitions = 0usize;
        while transitions < transition_limit {
            let gap = self.frames[self.current].gap;
            if gap.is_zero() {
                if !self.advance() {
                    break;
                }
                transitions += 1;
                continue;
            }

            if self.current == 0 && self.playback == Playback::Running {
                self.skip_whole_cycles(now);
            }
            let gap = self.frames[self.current].gap;
            let Some(deadline) = self.shown_at.checked_add(gap) else {
                break;
            };
            if now < deadline {
                break;
            }

            self.shown_at = deadline;
            if !self.advance() {
                break;
            }
            transitions += 1;
        }

        self.current().id != previous_id
    }

    /// Return the next time at which `tick` may change the visible frame.
    pub fn deadline(&self) -> Option<Instant> {
        if self.playback == Playback::Stopped || self.frames.len() < 2 || !self.has_timed_frame() {
            return None;
        }

        let gap = self.frames[self.current].gap;
        if gap.is_zero() {
            // Zero-gap frames must be advanced immediately, rather than after
            // the following visible frame's duration.
            return self.can_advance().then_some(self.shown_at);
        }
        self.can_advance()
            .then(|| self.shown_at.checked_add(gap))
            .flatten()
    }

    fn canvas_geometry(&self, controls: &Controls) -> Result<(u32, u32, usize), ProtocolError> {
        let Some(root) = self.frames.first() else {
            return Err(Self::error(
                controls,
                "ENOENT",
                "image has no animation frames",
            ));
        };
        let Some(bytes) = Self::pixel_len(root.image.width, root.image.height) else {
            return Err(Self::error(
                controls,
                "EINVAL",
                "invalid animation canvas dimensions",
            ));
        };
        if root.image.rgba.len() != bytes {
            return Err(Self::error(
                controls,
                "EINVAL",
                "invalid root frame pixel buffer",
            ));
        }
        Ok((root.image.width, root.image.height, bytes))
    }

    fn frame_index(
        &self,
        number: u32,
        controls: &Controls,
        name: &str,
    ) -> Result<usize, ProtocolError> {
        if number == 0 || number as usize > self.frames.len() {
            return Err(Self::error(
                controls,
                "ENOENT",
                format!("{name} frame does not exist"),
            ));
        }
        Ok(number as usize - 1)
    }

    fn validate_image(image: &DecodedImage, controls: &Controls) -> Result<(), ProtocolError> {
        let Some(bytes) = Self::pixel_len(image.width, image.height) else {
            return Err(Self::error(controls, "EINVAL", "invalid frame dimensions"));
        };
        if image.rgba.len() != bytes {
            return Err(Self::error(
                controls,
                "EINVAL",
                "invalid frame pixel buffer",
            ));
        }
        Ok(())
    }

    fn pixel_len(width: u32, height: u32) -> Option<usize> {
        if width == 0 || height == 0 {
            return None;
        }
        (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)
    }

    fn validate_rectangle(
        canvas_width: u32,
        canvas_height: u32,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        controls: &Controls,
        name: &str,
    ) -> Result<(), ProtocolError> {
        if width == 0
            || height == 0
            || (x as u64).saturating_add(width as u64) > canvas_width as u64
            || (y as u64).saturating_add(height as u64) > canvas_height as u64
        {
            return Err(Self::error(
                controls,
                "EINVAL",
                format!("{name} rectangle is out of bounds"),
            ));
        }
        Ok(())
    }

    fn rectangles_overlap(x1: u32, y1: u32, x2: u32, y2: u32, width: u32, height: u32) -> bool {
        let x1_end = x1 as u64 + width as u64;
        let y1_end = y1 as u64 + height as u64;
        let x2_end = x2 as u64 + width as u64;
        let y2_end = y2 as u64 + height as u64;
        u64::from(x1) < x2_end
            && u64::from(x2) < x1_end
            && u64::from(y1) < y2_end
            && u64::from(y2) < y1_end
    }

    fn background(len: usize, rgba: u32) -> Vec<u8> {
        let color = rgba.to_be_bytes();
        let mut pixels = vec![0; len];
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&color);
        }
        pixels
    }

    fn new_frame_gap(controls: &Controls) -> Duration {
        match controls.get(b'z') {
            Some(value) if value != 0 => Self::gap(value),
            _ => DEFAULT_GAP,
        }
    }

    fn gap(value: i64) -> Duration {
        if value <= 0 {
            Duration::ZERO
        } else {
            Duration::from_millis(value as u64)
        }
    }

    fn overlay(
        destination: &mut [u8],
        destination_width: u32,
        destination_x: u32,
        destination_y: u32,
        source: &[u8],
        source_width: u32,
        source_height: u32,
        replacement: bool,
    ) {
        Self::overlay_region(
            destination,
            destination_width,
            destination_x,
            destination_y,
            source,
            source_width,
            0,
            0,
            source_width,
            source_height,
            replacement,
        );
    }

    fn overlay_region(
        destination: &mut [u8],
        stride: u32,
        destination_x: u32,
        destination_y: u32,
        source: &[u8],
        source_stride: u32,
        source_x: u32,
        source_y: u32,
        width: u32,
        height: u32,
        replacement: bool,
    ) {
        for row in 0..height as usize {
            let destination_start =
                ((destination_y as usize + row) * stride as usize + destination_x as usize) * 4;
            let source_start =
                ((source_y as usize + row) * source_stride as usize + source_x as usize) * 4;
            let destination_row =
                &mut destination[destination_start..destination_start + width as usize * 4];
            let source_row = &source[source_start..source_start + width as usize * 4];
            if replacement {
                destination_row.copy_from_slice(source_row);
            } else {
                for (destination_pixel, source_pixel) in destination_row
                    .chunks_exact_mut(4)
                    .zip(source_row.chunks_exact(4))
                {
                    Self::blend(destination_pixel, source_pixel);
                }
            }
        }
    }

    /// Source-over composition for straight-alpha RGBA pixels.
    fn blend(destination: &mut [u8], source: &[u8]) {
        let source_alpha = source[3] as u32;
        if source_alpha == 255 {
            destination.copy_from_slice(source);
            return;
        }
        if source_alpha == 0 {
            return;
        }

        let destination_alpha = destination[3] as u32;
        let inverse_source_alpha = 255 - source_alpha;
        let output_alpha_numerator = source_alpha * 255 + destination_alpha * inverse_source_alpha;
        if output_alpha_numerator == 0 {
            destination.fill(0);
            return;
        }
        for channel in 0..3 {
            let numerator = source[channel] as u32 * source_alpha * 255
                + destination[channel] as u32 * destination_alpha * inverse_source_alpha;
            destination[channel] =
                ((numerator + output_alpha_numerator / 2) / output_alpha_numerator) as u8;
        }
        destination[3] = ((output_alpha_numerator + 127) / 255) as u8;
    }

    fn has_timed_frame(&self) -> bool {
        self.frames.iter().any(|frame| !frame.gap.is_zero())
    }

    fn can_advance(&self) -> bool {
        if self.frames.len() < 2 {
            return false;
        }
        if self.current + 1 < self.frames.len() {
            return true;
        }
        match self.playback {
            Playback::Running => self
                .maximum_loops
                .is_none_or(|maximum| self.completed_loops < maximum),
            Playback::Stopped | Playback::Loading => false,
        }
    }

    fn advance(&mut self) -> bool {
        if self.current + 1 < self.frames.len() {
            self.current += 1;
            return true;
        }
        if self.playback != Playback::Running {
            return false;
        }
        if let Some(maximum) = self.maximum_loops {
            if self.completed_loops >= maximum {
                return false;
            }
            self.completed_loops += 1;
        }
        self.current = 0;
        true
    }

    fn skip_whole_cycles(&mut self, now: Instant) {
        if self.current != 0 || self.playback != Playback::Running || now <= self.shown_at {
            return;
        }
        let cycle = self.cycle_duration();
        if cycle.is_zero() {
            return;
        }
        let elapsed = now.duration_since(self.shown_at);
        let cycle_nanos = cycle.as_nanos();
        let whole_cycles = elapsed.as_nanos() / cycle_nanos;
        if whole_cycles == 0 {
            return;
        }

        let cycles = match self.maximum_loops {
            Some(maximum) => whole_cycles.min((maximum - self.completed_loops) as u128),
            None => whole_cycles,
        };
        if cycles == 0 {
            return;
        }
        let Some(advance_by) = Self::duration_from_nanos(cycle_nanos.saturating_mul(cycles)) else {
            return;
        };
        let Some(shown_at) = self.shown_at.checked_add(advance_by) else {
            return;
        };
        self.shown_at = shown_at;
        if self.maximum_loops.is_some() {
            self.completed_loops += cycles as u32;
        }
    }

    fn cycle_duration(&self) -> Duration {
        let nanos = self.frames.iter().fold(0u128, |total, frame| {
            total.saturating_add(frame.gap.as_nanos())
        });
        Self::duration_from_nanos(nanos).unwrap_or(Duration::MAX)
    }

    fn duration_from_nanos(nanos: u128) -> Option<Duration> {
        let seconds = nanos / 1_000_000_000;
        let nanos = (nanos % 1_000_000_000) as u32;
        u64::try_from(seconds)
            .ok()
            .map(|seconds| Duration::new(seconds, nanos))
    }

    fn error(controls: &Controls, code: &'static str, message: impl Into<String>) -> ProtocolError {
        ProtocolError::new(code, message, controls.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controls(values: &[(u8, i64)]) -> Controls {
        let mut controls = Controls::default();
        for &(key, value) in values {
            controls.set(key, value);
        }
        controls
    }

    fn image(width: u32, height: u32, pixels: &[u8]) -> DecodedImage {
        DecodedImage {
            width,
            height,
            rgba: Arc::new(pixels.to_vec()),
        }
    }

    fn root(width: u32, height: u32, pixels: &[u8]) -> FrameImage {
        FrameImage {
            id: 1,
            width,
            height,
            rgba: Arc::new(pixels.to_vec()),
        }
    }

    #[test]
    fn composition_blends_and_replaces_with_fresh_resource_ids() {
        let now = Instant::now();
        let mut frames = Frames::new(root(1, 1, &[0, 0, 0, 255]));
        frames
            .transmit(&controls(&[]), image(1, 1, &[255, 0, 0, 128]), 2, now, 8)
            .unwrap();
        frames
            .compose(&controls(&[(b'r', 2), (b'c', 1)]), 3, now, 8)
            .unwrap();
        assert_eq!(frames.current().id, 3);
        assert_eq!(frames.current().rgba.as_slice(), &[128, 0, 0, 255]);

        frames
            .compose(&controls(&[(b'r', 2), (b'c', 1), (b'C', 1)]), 4, now, 8)
            .unwrap();
        assert_eq!(frames.current().id, 4);
        assert_eq!(frames.current().rgba.as_slice(), &[255, 0, 0, 128]);
    }

    #[test]
    fn invalid_composition_is_transactional() {
        let now = Instant::now();
        let mut frames = Frames::new(root(2, 1, &[0; 8]));
        let before_id = frames.current().id;
        let before_bytes = frames.bytes();
        let error = frames
            .compose(&controls(&[(b'r', 2), (b'c', 1)]), 2, now, 16)
            .unwrap_err();
        assert_eq!(error.code, "ENOENT");
        assert_eq!(frames.current().id, before_id);
        assert_eq!(frames.bytes(), before_bytes);
    }

    #[test]
    fn partial_frame_upload_uses_the_upload_stride_not_the_canvas_stride() {
        let now = Instant::now();
        let mut frames = Frames::new(root(2, 2, &[0; 16]));
        frames
            .transmit(
                &controls(&[(b'x', 1), (b'X', 1)]),
                image(1, 2, &[1, 2, 3, 255, 4, 5, 6, 255]),
                2,
                now,
                32,
            )
            .unwrap();
        assert_eq!(
            frames.images().nth(1).unwrap().rgba.as_slice(),
            &[0, 0, 0, 0, 1, 2, 3, 255, 0, 0, 0, 0, 4, 5, 6, 255]
        );
    }

    #[test]
    fn all_gapless_animation_has_no_busy_deadline() {
        let now = Instant::now();
        let mut frames = Frames::new(root(1, 1, &[0; 4]));
        frames
            .transmit(
                &controls(&[(b'z', -1)]),
                image(1, 1, &[1, 2, 3, 4]),
                2,
                now,
                8,
            )
            .unwrap();
        frames.control(&controls(&[(b's', 3)]), now).unwrap();
        assert!(!frames.tick(now + Duration::from_secs(60)));
        assert_eq!(frames.current().id, 1);
        assert_eq!(frames.deadline(), None);
    }

    #[test]
    fn catch_up_respects_a_finite_loop_count_and_stops_on_last_frame() {
        let now = Instant::now();
        let mut frames = Frames::new(root(1, 1, &[0; 4]));
        frames
            .control(&controls(&[(b'r', 1), (b'z', 10)]), now)
            .unwrap();
        frames
            .transmit(
                &controls(&[(b'z', 10)]),
                image(1, 1, &[10, 20, 30, 255]),
                2,
                now,
                8,
            )
            .unwrap();
        frames
            .control(&controls(&[(b's', 3), (b'v', 2)]), now)
            .unwrap();
        assert!(frames.tick(now + Duration::from_millis(55)));
        assert_eq!(frames.current().id, 2);
        assert_eq!(frames.deadline(), None);
    }

    #[test]
    fn uppercase_frame_deletion_removes_the_final_root() {
        let now = Instant::now();
        let mut frames = Frames::new(root(1, 1, &[0; 4]));
        frames
            .delete(&controls(&[(b'd', b'F' as i64)]), now)
            .unwrap();
        assert_eq!(frames.len(), 0);
        assert_eq!(frames.bytes(), 0);
    }
}
