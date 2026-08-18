use serde::{Deserialize, Serialize};
use yttt_core::model::ids::{ProjectId, TerminalSessionId};

use crate::path::ProjectRelativePath;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalGeometry {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteTerminalExecutionSpec {
    Shell { command: String },
    Command { program: String, args: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalExecutionSpec {
    Shell {
        program: String,
        args: Vec<String>,
        initial_command: Option<String>,
    },
    Command {
        shell: String,
        program: String,
        args: Vec<String>,
        return_to_shell: bool,
    },
    Ssh {
        connection_id: String,
        execution: RemoteTerminalExecutionSpec,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSpawnSpec {
    pub session_id: TerminalSessionId,
    pub project_id: ProjectId,
    pub cwd: ProjectRelativePath,
    pub execution: TerminalExecutionSpec,
    pub geometry: TerminalGeometry,
    pub geometry_epoch: u64,
    pub query_palette: Vec<u32>,
    pub palette_revision: u64,
    pub scrollback_limit: u32,
    pub environment: Vec<(String, String)>,
    pub removed_environment: Vec<String>,
}

impl TerminalSpawnSpec {
    pub fn address_fingerprint(&self) -> u64 {
        let encoded = postcard::to_allocvec(&(&self.project_id, &self.cwd, &self.execution))
            .expect("terminal spawn address must serialize");
        encoded
            .into_iter()
            .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalProcessState {
    Starting,
    Running,
    Exited { code: Option<i32> },
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SemanticColor {
    Named(u16),
    Indexed(u8),
    Rgb { red: u8, green: u8, blue: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticStyle {
    pub foreground: SemanticColor,
    pub background: SemanticColor,
    pub flags: u16,
    pub underline_color: SemanticColor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticSpan {
    pub start_column: u16,
    pub text: String,
    pub width: u16,
    pub style: SemanticStyle,
    pub hyperlink: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticRow {
    pub line_id: u64,
    pub viewport_row: u16,
    pub spans: Vec<SemanticSpan>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CursorShape {
    Block,
    Underline,
    Beam,
    HollowBlock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticCursor {
    pub row: u16,
    pub column: u16,
    pub shape: CursorShape,
    pub visible: bool,
    pub blinking: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicColor {
    pub index: u16,
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalPalette {
    pub colors: Vec<DynamicColor>,
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalModes {
    pub bits: u32,
    pub title: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticViewport {
    pub session_id: TerminalSessionId,
    pub session_epoch: u64,
    pub sequence: u64,
    pub geometry: TerminalGeometry,
    pub geometry_epoch: u64,
    pub scrollback_epoch: u64,
    pub history_size: u64,
    pub display_offset: u64,
    pub rows: Vec<SemanticRow>,
    pub cursor: SemanticCursor,
    pub modes: TerminalModes,
    pub palette: TerminalPalette,
    pub process_state: TerminalProcessState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticDelta {
    pub session_id: TerminalSessionId,
    pub session_epoch: u64,
    pub base_sequence: u64,
    pub sequence: u64,
    pub geometry_epoch: u64,
    pub scrollback_epoch: u64,
    pub history_size: u64,
    pub display_offset: u64,
    pub changed_rows: Vec<SemanticRow>,
    pub cursor: Option<SemanticCursor>,
    pub modes: Option<TerminalModes>,
    pub palette: Option<TerminalPalette>,
    pub process_state: Option<TerminalProcessState>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalCheckpoint {
    pub viewport: SemanticViewport,
    #[serde(with = "serde_bytes")]
    pub raw_replay_tail: Vec<u8>,
    pub raw_tail_start_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalStreamUpdate {
    Snapshot(SemanticViewport),
    Delta(SemanticDelta),
    RawTail {
        session_id: TerminalSessionId,
        session_epoch: u64,
        sequence: u64,
        #[serde(with = "serde_bytes")]
        bytes: Vec<u8>,
    },
    ResyncRequired {
        session_id: TerminalSessionId,
        available_from_sequence: u64,
    },
}

impl TerminalStreamUpdate {
    pub fn session_id(&self) -> &TerminalSessionId {
        match self {
            Self::Snapshot(viewport) => &viewport.session_id,
            Self::Delta(delta) => &delta.session_id,
            Self::RawTail { session_id, .. } | Self::ResyncRequired { session_id, .. } => {
                session_id
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalStreamApply {
    Updated(TerminalStreamDamage),
    Ignored,
    SequenceGap,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalStreamDamage {
    Full,
    Rows(Vec<u16>),
}

impl SemanticViewport {
    pub fn apply_stream_update(&mut self, update: TerminalStreamUpdate) -> TerminalStreamApply {
        match update {
            TerminalStreamUpdate::Snapshot(viewport) => {
                if viewport.session_epoch < self.session_epoch
                    || (viewport.session_epoch == self.session_epoch
                        && viewport.sequence < self.sequence)
                {
                    return TerminalStreamApply::Ignored;
                }
                *self = viewport;
                TerminalStreamApply::Updated(TerminalStreamDamage::Full)
            }
            TerminalStreamUpdate::Delta(delta) => self.apply_delta(delta),
            TerminalStreamUpdate::RawTail { .. } => TerminalStreamApply::Ignored,
            TerminalStreamUpdate::ResyncRequired { .. } => TerminalStreamApply::SequenceGap,
        }
    }

    pub fn apply_stream_update_ref(
        &mut self,
        update: &TerminalStreamUpdate,
    ) -> TerminalStreamApply {
        match update {
            TerminalStreamUpdate::Snapshot(viewport) => {
                if viewport.session_epoch < self.session_epoch
                    || (viewport.session_epoch == self.session_epoch
                        && viewport.sequence < self.sequence)
                {
                    return TerminalStreamApply::Ignored;
                }
                self.clone_from(viewport);
                TerminalStreamApply::Updated(TerminalStreamDamage::Full)
            }
            TerminalStreamUpdate::Delta(delta) => self.apply_delta_ref(delta),
            TerminalStreamUpdate::RawTail { .. } => TerminalStreamApply::Ignored,
            TerminalStreamUpdate::ResyncRequired { .. } => TerminalStreamApply::SequenceGap,
        }
    }

    fn apply_delta_ref(&mut self, delta: &SemanticDelta) -> TerminalStreamApply {
        if delta.sequence <= self.sequence {
            return TerminalStreamApply::Ignored;
        }
        if delta.session_id != self.session_id
            || delta.session_epoch != self.session_epoch
            || delta.base_sequence != self.sequence
        {
            return TerminalStreamApply::SequenceGap;
        }

        let full_damage = delta.geometry_epoch != self.geometry_epoch
            || delta.scrollback_epoch != self.scrollback_epoch
            || delta.display_offset != self.display_offset
            || delta.palette.is_some();
        let previous_cursor = self.cursor;
        let mut damaged_rows = delta
            .changed_rows
            .iter()
            .map(|row| row.viewport_row)
            .collect::<Vec<_>>();
        for row in &delta.changed_rows {
            if let Some(current) = self
                .rows
                .iter_mut()
                .find(|current| current.viewport_row == row.viewport_row)
            {
                current.clone_from(row);
            } else {
                self.rows.push(row.clone());
            }
        }
        self.rows.sort_by_key(|row| row.viewport_row);
        self.sequence = delta.sequence;
        self.geometry_epoch = delta.geometry_epoch;
        self.scrollback_epoch = delta.scrollback_epoch;
        self.history_size = delta.history_size;
        self.display_offset = delta.display_offset;
        if let Some(cursor) = delta.cursor {
            self.cursor = cursor;
        }
        if let Some(modes) = &delta.modes {
            self.modes.clone_from(modes);
        }
        if let Some(palette) = &delta.palette {
            self.palette.clone_from(palette);
        }
        if let Some(process_state) = delta.process_state {
            self.process_state = process_state;
        }

        if full_damage {
            return TerminalStreamApply::Updated(TerminalStreamDamage::Full);
        }
        if self.cursor != previous_cursor {
            damaged_rows.push(previous_cursor.row);
            damaged_rows.push(self.cursor.row);
        }
        damaged_rows.sort_unstable();
        damaged_rows.dedup();
        TerminalStreamApply::Updated(TerminalStreamDamage::Rows(damaged_rows))
    }

    fn apply_delta(&mut self, delta: SemanticDelta) -> TerminalStreamApply {
        if delta.sequence <= self.sequence {
            return TerminalStreamApply::Ignored;
        }
        if delta.session_id != self.session_id
            || delta.session_epoch != self.session_epoch
            || delta.base_sequence != self.sequence
        {
            return TerminalStreamApply::SequenceGap;
        }

        let full_damage = delta.geometry_epoch != self.geometry_epoch
            || delta.scrollback_epoch != self.scrollback_epoch
            || delta.display_offset != self.display_offset
            || delta.palette.is_some();
        let previous_cursor = self.cursor;
        let mut damaged_rows = delta
            .changed_rows
            .iter()
            .map(|row| row.viewport_row)
            .collect::<Vec<_>>();
        for row in delta.changed_rows {
            if let Some(current) = self
                .rows
                .iter_mut()
                .find(|current| current.viewport_row == row.viewport_row)
            {
                *current = row;
            } else {
                self.rows.push(row);
            }
        }
        self.rows.sort_by_key(|row| row.viewport_row);
        self.sequence = delta.sequence;
        self.geometry_epoch = delta.geometry_epoch;
        self.scrollback_epoch = delta.scrollback_epoch;
        self.history_size = delta.history_size;
        self.display_offset = delta.display_offset;
        if let Some(cursor) = delta.cursor {
            self.cursor = cursor;
        }
        if let Some(modes) = delta.modes {
            self.modes = modes;
        }
        if let Some(palette) = delta.palette {
            self.palette = palette;
        }
        if let Some(process_state) = delta.process_state {
            self.process_state = process_state;
        }

        if full_damage {
            return TerminalStreamApply::Updated(TerminalStreamDamage::Full);
        }
        if self.cursor != previous_cursor {
            damaged_rows.push(previous_cursor.row);
            damaged_rows.push(self.cursor.row);
        }
        damaged_rows.sort_unstable();
        damaged_rows.dedup();
        TerminalStreamApply::Updated(TerminalStreamDamage::Rows(damaged_rows))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalInput {
    pub session_id: TerminalSessionId,
    pub context: TerminalMutationContext,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalMutationContext {
    pub host_epoch: u64,
    pub session_epoch: u64,
    pub lease_epoch: u64,
    pub geometry_epoch: u64,
    pub client_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResizeTerminal {
    pub session_id: TerminalSessionId,
    pub context: TerminalMutationContext,
    pub geometry: TerminalGeometry,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrollTerminal {
    pub session_id: TerminalSessionId,
    pub context: TerminalMutationContext,
    pub display_offset: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetTerminalQueryPalette {
    pub session_id: TerminalSessionId,
    pub context: TerminalMutationContext,
    pub colors: Vec<u32>,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalResize {
    pub geometry: TerminalGeometry,
    pub geometry_epoch: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachTerminal {
    pub session_id: TerminalSessionId,
    pub known_session_epoch: Option<u64>,
    pub after_sequence: Option<u64>,
    pub mode: TerminalLeaseMode,
    pub geometry: TerminalGeometry,
    pub geometry_epoch: u64,
    pub query_palette: Vec<u32>,
    pub palette_revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalLeaseMode {
    Observer,
    Interactive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalViewportAnchor {
    Bottom,
    DisplayOffset(u64),
    LineId(u64),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadTerminalViewport {
    pub session_id: TerminalSessionId,
    pub session_epoch: u64,
    pub scrollback_epoch: u64,
    pub anchor: TerminalViewportAnchor,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalViewportRead {
    pub viewport: SemanticViewport,
    pub bottom_line_id: Option<u64>,
    pub checkpoint_sequence: u64,
    pub unseen_output: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchTerminal {
    pub session_id: TerminalSessionId,
    pub session_epoch: u64,
    pub scrollback_epoch: u64,
    pub generation: u64,
    pub query: String,
    pub case_sensitive: bool,
    pub max_results: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSearchMatch {
    pub line_id: u64,
    pub start_column: u16,
    pub end_column: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSearchResults {
    pub session_id: TerminalSessionId,
    pub session_epoch: u64,
    pub scrollback_epoch: u64,
    pub generation: u64,
    pub matches: Vec<TerminalSearchMatch>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminateTerminalRequest {
    pub request_id: u64,
    pub session_id: TerminalSessionId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminatedTerminal {
    pub session_epoch: u64,
    pub final_sequence: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminationMode {
    Detach,
    Terminate,
    TerminateMany,
}
