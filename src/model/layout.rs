#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ProjectLayout {
    pub project: ProjectConfig,
    #[serde(default)]
    pub tabs: Vec<TabConfig>,
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
