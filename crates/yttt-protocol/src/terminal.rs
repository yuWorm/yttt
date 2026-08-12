use serde::{Deserialize, Serialize};
use yttt_core::model::ids::{PaneId, ProjectId, TabId, TerminalSessionId};

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
    pub tab_id: TabId,
    pub pane_id: PaneId,
    pub cwd: String,
    pub execution: TerminalExecutionSpec,
    pub geometry: TerminalGeometry,
    pub geometry_epoch: u64,
    pub scrollback_limit: u32,
    pub environment: Vec<(String, String)>,
    pub removed_environment: Vec<String>,
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
        bytes: Vec<u8>,
    },
    ResyncRequired {
        session_id: TerminalSessionId,
        available_from_sequence: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalInput {
    pub session_id: TerminalSessionId,
    pub client_sequence: u64,
    pub bytes: Vec<u8>,
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
    pub geometry: TerminalGeometry,
    pub geometry_epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalLeaseMode {
    Observer,
    Interactive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminationMode {
    Detach,
    Terminate,
    TerminateMany,
}
