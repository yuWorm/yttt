use yttt_core::model::ids::TerminalSessionId;
use yttt_protocol::terminal::{SemanticDelta, SemanticViewport, TerminalStreamUpdate};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorApply {
    Updated,
    Ignored,
    SequenceGap,
}

#[derive(Clone, Debug)]
pub struct TerminalMirror {
    viewport: SemanticViewport,
}

impl TerminalMirror {
    pub fn new(viewport: SemanticViewport) -> Self {
        Self { viewport }
    }

    pub fn session_id(&self) -> &TerminalSessionId {
        &self.viewport.session_id
    }

    pub fn viewport(&self) -> &SemanticViewport {
        &self.viewport
    }

    pub fn apply(&mut self, update: TerminalStreamUpdate) -> MirrorApply {
        match update {
            TerminalStreamUpdate::Snapshot(viewport) => {
                if viewport.session_epoch < self.viewport.session_epoch
                    || (viewport.session_epoch == self.viewport.session_epoch
                        && viewport.sequence < self.viewport.sequence)
                {
                    return MirrorApply::Ignored;
                }
                self.viewport = viewport;
                MirrorApply::Updated
            }
            TerminalStreamUpdate::Delta(delta) => self.apply_delta(delta),
            TerminalStreamUpdate::RawTail { .. } | TerminalStreamUpdate::ResyncRequired { .. } => {
                MirrorApply::SequenceGap
            }
        }
    }

    fn apply_delta(&mut self, delta: SemanticDelta) -> MirrorApply {
        if delta.session_epoch != self.viewport.session_epoch {
            return MirrorApply::SequenceGap;
        }
        if delta.sequence <= self.viewport.sequence {
            return MirrorApply::Ignored;
        }
        if delta.base_sequence != self.viewport.sequence {
            return MirrorApply::SequenceGap;
        }
        for changed in delta.changed_rows {
            if let Some(row) = self
                .viewport
                .rows
                .iter_mut()
                .find(|row| row.viewport_row == changed.viewport_row)
            {
                *row = changed;
            } else {
                self.viewport.rows.push(changed);
            }
        }
        self.viewport.rows.sort_by_key(|row| row.viewport_row);
        self.viewport.sequence = delta.sequence;
        self.viewport.geometry_epoch = delta.geometry_epoch;
        self.viewport.scrollback_epoch = delta.scrollback_epoch;
        self.viewport.history_size = delta.history_size;
        self.viewport.display_offset = delta.display_offset;
        if let Some(cursor) = delta.cursor {
            self.viewport.cursor = cursor;
        }
        if let Some(modes) = delta.modes {
            self.viewport.modes = modes;
        }
        if let Some(palette) = delta.palette {
            self.viewport.palette = palette;
        }
        if let Some(process_state) = delta.process_state {
            self.viewport.process_state = process_state;
        }
        MirrorApply::Updated
    }
}
