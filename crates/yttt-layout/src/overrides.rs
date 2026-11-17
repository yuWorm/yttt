//! A personal patch over a layout, and how it merges.

use std::path::PathBuf;

use yttt_core::model::layout::{
    LayoutError, LayoutNode, PaneConfig, PaneKind, ProcessExitBehavior, ProjectLayout, TabStartup,
    TerminalExecutionMode,
};

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct LayoutOverride {
    pub project: Option<ProjectOverride>,
    #[serde(default)]
    pub tabs: Vec<TabOverride>,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ProjectOverride {
    pub name: Option<String>,
    pub default_tab: Option<String>,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct TabOverride {
    pub id: String,
    pub title: Option<String>,
    pub cwd: Option<PathBuf>,
    pub startup: Option<TabStartup>,
    pub layout: Option<LayoutNodeOverride>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutNodeOverride {
    Pane(PaneOverride),
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct PaneOverride {
    pub id: String,
    pub title: Option<String>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub execution_mode: Option<TerminalExecutionMode>,
    pub exit_behavior: Option<ProcessExitBehavior>,
    pub kind: Option<PaneKind>,
    pub notify_on_exit: Option<bool>,
    pub detector: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeWarning {
    StaleTabOverride(String),
    StalePaneOverride(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutMerge {
    pub layout: ProjectLayout,
    pub warnings: Vec<MergeWarning>,
}

pub fn merge_layouts(
    base: &ProjectLayout,
    local_override: &LayoutOverride,
) -> Result<LayoutMerge, LayoutError> {
    let mut layout = base.clone();
    let mut warnings = Vec::new();

    if let Some(project_override) = &local_override.project {
        if let Some(name) = &project_override.name {
            layout.project.name = name.clone();
        }
        if let Some(default_tab) = &project_override.default_tab {
            layout.project.default_tab = Some(default_tab.clone());
        }
    }

    for tab_override in &local_override.tabs {
        let Some(tab) = layout.tabs.iter_mut().find(|tab| tab.id == tab_override.id) else {
            warnings.push(MergeWarning::StaleTabOverride(tab_override.id.clone()));
            continue;
        };

        if let Some(title) = &tab_override.title {
            tab.title = title.clone();
            // A title in a personal patch is one a person chose.
            tab.renamed = true;
        }
        if let Some(cwd) = &tab_override.cwd {
            tab.cwd = Some(cwd.clone());
        }
        if let Some(startup) = tab_override.startup {
            tab.startup = startup;
        }

        if let Some(layout_override) = &tab_override.layout {
            match layout_override {
                LayoutNodeOverride::Pane(pane_override) => {
                    if !apply_pane_override(&mut tab.layout, pane_override) {
                        warnings.push(MergeWarning::StalePaneOverride(pane_override.id.clone()));
                    }
                }
            }
        }
    }

    layout.validate()?;

    Ok(LayoutMerge { layout, warnings })
}

fn apply_pane_override(layout: &mut LayoutNode, pane_override: &PaneOverride) -> bool {
    let Some(pane) = layout.find_pane_mut(&pane_override.id) else {
        return false;
    };

    merge_pane(pane, pane_override);
    true
}

fn merge_pane(pane: &mut PaneConfig, pane_override: &PaneOverride) {
    if let Some(title) = &pane_override.title {
        pane.title = title.clone();
    }
    if let Some(command) = &pane_override.command {
        pane.command = command.clone();
    }
    if let Some(args) = &pane_override.args {
        pane.args = args.clone();
    }
    if let Some(execution_mode) = pane_override.execution_mode {
        pane.execution_mode = execution_mode;
    }
    if let Some(exit_behavior) = pane_override.exit_behavior {
        pane.exit_behavior = exit_behavior;
    }
    if let Some(kind) = &pane_override.kind {
        pane.kind = kind.clone();
    }
    if let Some(notify_on_exit) = pane_override.notify_on_exit {
        pane.notify_on_exit = notify_on_exit;
    }
    if let Some(detector) = &pane_override.detector {
        pane.detector = Some(detector.clone());
    }
}
