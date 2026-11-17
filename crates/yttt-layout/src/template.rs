//! The layout a project gets when it has none of its own.

use std::path::Path;

use yttt_core::model::layout::{
    LayoutError, LayoutNode, PaneConfig, PaneKind, ProcessExitBehavior, ProjectConfig,
    ProjectLayout, SplitConfig, SplitDirection, TabConfig, TabStartup, TerminalExecutionMode,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuiltinAgent {
    #[default]
    Codex,
    Claude,
    Grok,
    OpenCode,
    Pi,
    #[serde(rename = "omp")]
    OhMyPi,
}

impl BuiltinAgent {
    pub const ALL: [Self; 6] = [
        Self::Codex,
        Self::Claude,
        Self::Grok,
        Self::OpenCode,
        Self::Pi,
        Self::OhMyPi,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Grok => "grok",
            Self::OpenCode => "opencode",
            Self::Pi => "pi",
            Self::OhMyPi => "omp",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "codex" => Some(Self::Codex),
            "claude" | "claude-code" => Some(Self::Claude),
            "grok" | "groky" => Some(Self::Grok),
            "opencode" | "open-code" => Some(Self::OpenCode),
            "pi" => Some(Self::Pi),
            "omp" | "oh-my-pi" => Some(Self::OhMyPi),
            _ => None,
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude Code",
            Self::Grok => "Grok",
            Self::OpenCode => "OpenCode",
            Self::Pi => "Pi",
            Self::OhMyPi => "Oh My Pi",
        }
    }

    pub const fn command(self) -> &'static str {
        self.id()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DefaultLayoutKind {
    #[default]
    SplitPane,
    SeparateTabs,
}

impl DefaultLayoutKind {
    pub const ALL: [Self; 2] = [Self::SplitPane, Self::SeparateTabs];

    pub const fn id(self) -> &'static str {
        match self {
            Self::SplitPane => "split",
            Self::SeparateTabs => "tabs",
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DefaultLayoutTemplate {
    pub project: DefaultProjectTemplate,
    #[serde(default)]
    pub tabs: Vec<TabConfig>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DefaultProjectTemplate {
    pub default_tab: Option<String>,
}

impl DefaultLayoutTemplate {
    pub fn builtin() -> Self {
        Self {
            project: DefaultProjectTemplate {
                default_tab: Some("shell".to_string()),
            },
            tabs: vec![TabConfig {
                renamed: false,
                id: "shell".to_string(),
                title: "Shell".to_string(),
                cwd: None,
                startup: TabStartup::Lazy,
                layout: LayoutNode::Pane(shell_pane()),
            }],
        }
    }

    pub fn for_onboarding(layout_kind: DefaultLayoutKind, agent: BuiltinAgent) -> Self {
        match layout_kind {
            DefaultLayoutKind::SplitPane => Self {
                project: DefaultProjectTemplate {
                    default_tab: Some("workspace".to_string()),
                },
                tabs: vec![TabConfig {
                    renamed: false,
                    id: "workspace".to_string(),
                    title: "Workspace".to_string(),
                    cwd: None,
                    startup: TabStartup::Lazy,
                    layout: LayoutNode::Split(SplitConfig {
                        direction: SplitDirection::Horizontal,
                        ratio: 0.65,
                        left: Box::new(LayoutNode::Pane(agent_pane(agent))),
                        right: Box::new(LayoutNode::Pane(shell_pane())),
                    }),
                }],
            },
            DefaultLayoutKind::SeparateTabs => Self {
                project: DefaultProjectTemplate {
                    default_tab: Some("agent".to_string()),
                },
                tabs: vec![
                    TabConfig {
                        renamed: false,
                        id: "agent".to_string(),
                        title: agent.display_name().to_string(),
                        cwd: None,
                        startup: TabStartup::Lazy,
                        layout: LayoutNode::Pane(agent_pane(agent)),
                    },
                    TabConfig {
                        renamed: false,
                        id: "shell".to_string(),
                        title: "Shell".to_string(),
                        cwd: None,
                        startup: TabStartup::Lazy,
                        layout: LayoutNode::Pane(shell_pane()),
                    },
                ],
            },
        }
    }

    pub fn primary_agent(&self) -> Option<BuiltinAgent> {
        self.tabs
            .iter()
            .find_map(|tab| primary_agent_in_layout(&tab.layout))
    }

    pub fn materialize(&self, project_name: impl Into<String>) -> ProjectLayout {
        ProjectLayout {
            project: ProjectConfig {
                name: project_name.into(),
                default_tab: self.project.default_tab.clone(),
            },
            tabs: self.tabs.clone(),
        }
    }

    pub fn validate(&self) -> Result<(), LayoutError> {
        self.materialize("Project").validate()
    }
}

fn primary_agent_in_layout(layout: &LayoutNode) -> Option<BuiltinAgent> {
    match layout {
        LayoutNode::Pane(pane) => {
            let command = pane.command.split_whitespace().next().unwrap_or_default();
            let command = Path::new(command)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(command);
            BuiltinAgent::from_id(command)
        }
        LayoutNode::Split(split) => {
            primary_agent_in_layout(&split.left).or_else(|| primary_agent_in_layout(&split.right))
        }
    }
}

fn agent_pane(agent: BuiltinAgent) -> PaneConfig {
    PaneConfig {
        id: agent.id().to_string(),
        title: agent.display_name().to_string(),
        command: agent.command().to_string(),
        args: Vec::new(),
        execution_mode: TerminalExecutionMode::Command,
        exit_behavior: ProcessExitBehavior::ManualRestart,
        kind: PaneKind::Agent,
        notify_on_exit: true,
        detector: None,
        role: yttt_core::model::layout::PaneRole::Worker,
        spawned_by: None,
        controller: yttt_core::model::layout::PaneController::Auto,
    }
}

fn shell_pane() -> PaneConfig {
    PaneConfig {
        id: "shell".to_string(),
        title: "Shell".to_string(),
        command: String::new(),
        args: Vec::new(),
        execution_mode: TerminalExecutionMode::Shell,
        exit_behavior: ProcessExitBehavior::Close,
        kind: PaneKind::Shell,
        notify_on_exit: false,
        detector: None,
        role: yttt_core::model::layout::PaneRole::Worker,
        spawned_by: None,
        controller: yttt_core::model::layout::PaneController::Auto,
    }
}

/// Why a default-layout file was not used.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    #[error("{0}")]
    Parse(String),
    #[error("{0}")]
    Validation(String),
}

/// Parses and validates a `default-layout.toml`.
pub fn parse_template(source: &str) -> Result<DefaultLayoutTemplate, TemplateError> {
    let template = toml::from_str::<DefaultLayoutTemplate>(source)
        .map_err(|error| TemplateError::Parse(error.to_string()))?;
    template
        .validate()
        .map_err(|error| TemplateError::Validation(error.to_string()))?;
    Ok(template)
}
