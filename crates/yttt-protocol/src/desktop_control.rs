use serde::{Deserialize, Serialize};
use yttt_agent_core::AgentSnapshot;

pub const MAX_CONTROL_TEXT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopControlRequest {
    pub window: Option<String>,
    pub project: Option<String>,
    pub tab: Option<String>,
    pub pane: Option<String>,
    pub command: DesktopControlCommand,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopControlCommand {
    Windows,
    Projects,
    Tabs,
    Panes,
    Agents,
    CreateShell {
        command: String,
        title: Option<String>,
    },
    CreateAgent {
        provider: String,
        args: Vec<String>,
        title: Option<String>,
    },
    Split {
        direction: ControlSplitDirection,
        command: String,
    },
    Focus,
    Rename {
        title: String,
    },
    Close,
    Resize {
        direction: ControlResizeDirection,
        percent: u8,
    },
    Send {
        text: String,
        enter: bool,
        raw: bool,
        agent_only: bool,
    },
    Read,
}

impl DesktopControlCommand {
    pub fn is_list(&self) -> bool {
        matches!(
            self,
            Self::Windows | Self::Projects | Self::Tabs | Self::Panes | Self::Agents
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlSplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlResizeDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlTarget {
    pub window: String,
    pub project: String,
    pub tab: String,
    pub pane: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlWindow {
    pub id: String,
    pub selected_project: Option<String>,
    pub writable: bool,
    pub loading: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlProject {
    pub window: String,
    pub id: String,
    pub name: String,
    pub path: String,
    pub selected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlTab {
    pub target: ControlTarget,
    pub title: String,
    pub selected: bool,
    pub panes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPane {
    pub target: ControlTarget,
    pub title: String,
    pub command: String,
    pub session_id: String,
    pub state: String,
    pub focused: bool,
    pub agent: Option<AgentSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopControlResponse {
    Windows(Vec<ControlWindow>),
    Projects(Vec<ControlProject>),
    Tabs(Vec<ControlTab>),
    Panes(Vec<ControlPane>),
    Agents(Vec<ControlPane>),
    Created {
        target: ControlTarget,
        state: String,
    },
    Updated {
        target: ControlTarget,
    },
    Closed {
        target: ControlTarget,
    },
    InputAccepted {
        target: ControlTarget,
        bytes: usize,
    },
    Text {
        target: ControlTarget,
        text: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopControlErrorCode {
    InvalidRequest,
    NotFound,
    AmbiguousTarget,
    NotReady,
    PermissionDenied,
    Busy,
    OutcomeUnknown,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopControlError {
    pub code: DesktopControlErrorCode,
    pub message: String,
}

impl DesktopControlError {
    pub fn new(code: DesktopControlErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub type DesktopControlResult = Result<DesktopControlResponse, DesktopControlError>;

impl DesktopControlRequest {
    pub fn validate(&self) -> Result<(), DesktopControlError> {
        let invalid =
            |message| DesktopControlError::new(DesktopControlErrorCode::InvalidRequest, message);
        for value in [&self.window, &self.project, &self.tab, &self.pane]
            .into_iter()
            .flatten()
        {
            if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
                return Err(invalid("Target identifiers must be nonempty, bounded text"));
            }
        }
        if self.pane.is_some() && self.tab.is_none() || self.tab.is_some() && self.project.is_none()
        {
            return Err(invalid(
                "A pane requires --tab and --project; a tab requires --project",
            ));
        }
        let text = match &self.command {
            DesktopControlCommand::CreateShell { command, title } => {
                validate_title(title.as_deref())?;
                Some(command.as_str())
            }
            DesktopControlCommand::CreateAgent {
                provider,
                args,
                title,
            } => {
                validate_title(title.as_deref())?;
                if provider.is_empty()
                    || provider.len() > 64
                    || args.len() > 128
                    || args.iter().map(String::len).sum::<usize>() > MAX_CONTROL_TEXT_BYTES
                    || args.iter().any(|arg| arg.contains('\0'))
                {
                    return Err(invalid("Invalid Agent provider or arguments"));
                }
                None
            }
            DesktopControlCommand::Split { command, .. } => Some(command.as_str()),
            DesktopControlCommand::Send { text, .. } => Some(text.as_str()),
            DesktopControlCommand::Rename { title } => {
                validate_title(Some(title))?;
                None
            }
            DesktopControlCommand::Resize { percent, .. } if !(1..=90).contains(percent) => {
                return Err(invalid("Resize percent must be between 1 and 90"));
            }
            _ => None,
        };
        if text.is_some_and(|text| text.len() > MAX_CONTROL_TEXT_BYTES || text.contains('\0')) {
            return Err(invalid(
                "Input must be at most 64 KiB and contain no NUL bytes",
            ));
        }
        if self.command.is_list() {
            return Ok(());
        }
        if self.project.is_none() {
            return Err(invalid("--project is required"));
        }
        if matches!(
            self.command,
            DesktopControlCommand::CreateShell { .. } | DesktopControlCommand::CreateAgent { .. }
        ) {
            if self.tab.is_some() || self.pane.is_some() {
                return Err(invalid(
                    "Creating a tab targets a project, not an existing tab or pane",
                ));
            }
            return Ok(());
        }
        if self.tab.is_none() {
            return Err(invalid("--tab is required"));
        }
        if matches!(
            self.command,
            DesktopControlCommand::Split { .. }
                | DesktopControlCommand::Resize { .. }
                | DesktopControlCommand::Send { .. }
                | DesktopControlCommand::Read
        ) && self.pane.is_none()
        {
            return Err(invalid("--pane is required"));
        }
        Ok(())
    }
}

fn validate_title(title: Option<&str>) -> Result<(), DesktopControlError> {
    if title.is_some_and(|title| {
        title.trim().is_empty() || title.len() > 1024 || title.chars().any(char::is_control)
    }) {
        return Err(DesktopControlError::new(
            DesktopControlErrorCode::InvalidRequest,
            "Titles must be nonempty, at most 1024 bytes, and contain no control characters",
        ));
    }
    Ok(())
}
