use std::collections::HashSet;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ProjectLayout {
    pub project: ProjectConfig,
    #[serde(default)]
    pub tabs: Vec<TabConfig>,
}

impl ProjectLayout {
    pub fn validate(&self) -> Result<(), LayoutError> {
        let mut tab_ids = HashSet::new();

        for tab in &self.tabs {
            if !tab_ids.insert(&tab.id) {
                return Err(LayoutError::DuplicateTabId(tab.id.clone()));
            }

            tab.layout.validate()?;
        }

        if let Some(default_tab) = &self.project.default_tab {
            if !tab_ids.contains(default_tab) {
                return Err(LayoutError::MissingDefaultTab(default_tab.clone()));
            }
        }

        Ok(())
    }

    pub fn tab(&self, id: &str) -> Option<&TabConfig> {
        self.tabs.iter().find(|tab| tab.id == id)
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ProjectConfig {
    pub name: String,
    pub default_tab: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct TabConfig {
    pub id: String,
    pub title: String,
    pub layout: LayoutNode,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutNode {
    Pane(PaneConfig),
    Split(SplitConfig),
}

impl LayoutNode {
    pub fn pane_id(&self) -> Option<&str> {
        match self {
            Self::Pane(pane) => Some(&pane.id),
            Self::Split(_) => None,
        }
    }

    fn validate(&self) -> Result<(), LayoutError> {
        match self {
            Self::Pane(_) => Ok(()),
            Self::Split(split) => {
                if !(0.05..=0.95).contains(&split.ratio) {
                    return Err(LayoutError::InvalidSplitRatio(split.ratio));
                }

                split.left.validate()?;
                split.right.validate()
            }
        }
    }

    pub fn find_pane(&self, id: &str) -> Option<&PaneConfig> {
        match self {
            Self::Pane(pane) if pane.id == id => Some(pane),
            Self::Pane(_) => None,
            Self::Split(split) => split
                .left
                .find_pane(id)
                .or_else(|| split.right.find_pane(id)),
        }
    }

    pub(crate) fn find_pane_mut(&mut self, id: &str) -> Option<&mut PaneConfig> {
        match self {
            Self::Pane(pane) if pane.id == id => Some(pane),
            Self::Pane(_) => None,
            Self::Split(split) => {
                if let Some(pane) = split.left.find_pane_mut(id) {
                    Some(pane)
                } else {
                    split.right.find_pane_mut(id)
                }
            }
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct PaneConfig {
    pub id: String,
    pub title: String,
    pub command: String,
    #[serde(default)]
    pub kind: PaneKind,
    #[serde(default)]
    pub notify_on_exit: bool,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct SplitConfig {
    pub direction: SplitDirection,
    pub ratio: f32,
    pub left: Box<LayoutNode>,
    pub right: Box<LayoutNode>,
}

#[derive(Clone, Copy, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PaneKind {
    #[default]
    Shell,
    Agent,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum LayoutError {
    #[error("duplicate tab id: {0}")]
    DuplicateTabId(String),
    #[error("default tab does not exist: {0}")]
    MissingDefaultTab(String),
    #[error("split ratio must be between 0.05 and 0.95, got {0}")]
    InvalidSplitRatio(f32),
}
