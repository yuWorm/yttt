use crate::{
    config::layout_loader::{
        LayoutNodeOverride, LayoutOverride, PaneOverride, ProjectOverride, TabOverride,
    },
    model::layout::{
        LayoutNode, PaneConfig, PaneKind, ProjectConfig, ProjectLayout, SplitConfig,
        SplitDirection, TabConfig,
    },
};

const PERSONAL_LAYOUT_VERSION: i64 = 1;

#[derive(Clone, Debug, PartialEq)]
pub enum PersonalLayout {
    Patch(LayoutOverride),
    Replace(ProjectLayout),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PersonalLayoutFileError {
    Parse(String),
    Validation(String),
}

pub(crate) fn parse(source: &str) -> Result<PersonalLayout, PersonalLayoutFileError> {
    let value: toml::Value = toml::from_str(source)
        .map_err(|error| PersonalLayoutFileError::Parse(error.to_string()))?;
    let table = value.as_table().ok_or_else(|| {
        PersonalLayoutFileError::Validation("personal layout must be a TOML table".to_string())
    })?;

    let version = table
        .get("version")
        .and_then(toml::Value::as_integer)
        .ok_or_else(|| {
            PersonalLayoutFileError::Validation(
                "personal layout requires integer version = 1".to_string(),
            )
        })?;
    if version != PERSONAL_LAYOUT_VERSION {
        return Err(PersonalLayoutFileError::Validation(format!(
            "unsupported personal layout version: {version}"
        )));
    }

    let mode = table
        .get("mode")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| {
            PersonalLayoutFileError::Validation(
                "personal layout requires mode = \"patch\" or \"replace\"".to_string(),
            )
        })?;
    if !table.contains_key("layout") {
        return Err(PersonalLayoutFileError::Validation(
            "personal layout requires a layout body".to_string(),
        ));
    }

    match mode {
        "patch" => {
            let file: PatchFileV1 = toml::from_str(source).map_err(validation_error)?;
            Ok(PersonalLayout::Patch(file.layout.into()))
        }
        "replace" => {
            let file: ReplaceFileV1 = toml::from_str(source).map_err(validation_error)?;
            let layout: ProjectLayout = file.layout.into();
            layout
                .validate()
                .map_err(|error| PersonalLayoutFileError::Validation(error.to_string()))?;
            Ok(PersonalLayout::Replace(layout))
        }
        other => Err(PersonalLayoutFileError::Validation(format!(
            "unsupported personal layout mode: {other}"
        ))),
    }
}

pub(crate) fn serialize_patch(layout: &LayoutOverride) -> Result<String, toml::ser::Error> {
    toml::to_string_pretty(&PatchFileV1 {
        version: PERSONAL_LAYOUT_VERSION as u32,
        mode: "patch".to_string(),
        layout: StrictPatchLayout::from(layout),
    })
}

pub(crate) fn serialize_replace(layout: &ProjectLayout) -> Result<String, toml::ser::Error> {
    toml::to_string_pretty(&ReplaceFileV1 {
        version: PERSONAL_LAYOUT_VERSION as u32,
        mode: "replace".to_string(),
        layout: StrictProjectLayout::from(layout),
    })
}

fn validation_error(error: toml::de::Error) -> PersonalLayoutFileError {
    PersonalLayoutFileError::Validation(error.to_string())
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct PatchFileV1 {
    version: u32,
    mode: String,
    layout: StrictPatchLayout,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct ReplaceFileV1 {
    version: u32,
    mode: String,
    layout: StrictProjectLayout,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct StrictPatchLayout {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project: Option<StrictProjectOverride>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tabs: Vec<StrictTabOverride>,
}

impl From<StrictPatchLayout> for LayoutOverride {
    fn from(value: StrictPatchLayout) -> Self {
        Self {
            project: value.project.map(Into::into),
            tabs: value.tabs.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<&LayoutOverride> for StrictPatchLayout {
    fn from(value: &LayoutOverride) -> Self {
        Self {
            project: value.project.as_ref().map(Into::into),
            tabs: value.tabs.iter().map(Into::into).collect(),
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct StrictProjectOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_tab: Option<String>,
}

impl From<StrictProjectOverride> for ProjectOverride {
    fn from(value: StrictProjectOverride) -> Self {
        Self {
            name: value.name,
            default_tab: value.default_tab,
        }
    }
}

impl From<&ProjectOverride> for StrictProjectOverride {
    fn from(value: &ProjectOverride) -> Self {
        Self {
            name: value.name.clone(),
            default_tab: value.default_tab.clone(),
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct StrictTabOverride {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layout: Option<StrictLayoutNodeOverride>,
}

impl From<StrictTabOverride> for TabOverride {
    fn from(value: StrictTabOverride) -> Self {
        Self {
            id: value.id,
            title: value.title,
            layout: value.layout.map(Into::into),
        }
    }
}

impl From<&TabOverride> for StrictTabOverride {
    fn from(value: &TabOverride) -> Self {
        Self {
            id: value.id.clone(),
            title: value.title.clone(),
            layout: value.layout.as_ref().map(Into::into),
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StrictLayoutNodeOverride {
    Pane(StrictPaneOverride),
}

impl From<StrictLayoutNodeOverride> for LayoutNodeOverride {
    fn from(value: StrictLayoutNodeOverride) -> Self {
        match value {
            StrictLayoutNodeOverride::Pane(pane) => Self::Pane(pane.into()),
        }
    }
}

impl From<&LayoutNodeOverride> for StrictLayoutNodeOverride {
    fn from(value: &LayoutNodeOverride) -> Self {
        match value {
            LayoutNodeOverride::Pane(pane) => Self::Pane(pane.into()),
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct StrictPaneOverride {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kind: Option<PaneKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    notify_on_exit: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    detector: Option<String>,
}

impl From<StrictPaneOverride> for PaneOverride {
    fn from(value: StrictPaneOverride) -> Self {
        Self {
            id: value.id,
            title: value.title,
            command: value.command,
            kind: value.kind,
            notify_on_exit: value.notify_on_exit,
            detector: value.detector,
        }
    }
}

impl From<&PaneOverride> for StrictPaneOverride {
    fn from(value: &PaneOverride) -> Self {
        Self {
            id: value.id.clone(),
            title: value.title.clone(),
            command: value.command.clone(),
            kind: value.kind.clone(),
            notify_on_exit: value.notify_on_exit,
            detector: value.detector.clone(),
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct StrictProjectLayout {
    project: StrictProjectConfig,
    tabs: Vec<StrictTabConfig>,
}

impl From<StrictProjectLayout> for ProjectLayout {
    fn from(value: StrictProjectLayout) -> Self {
        Self {
            project: value.project.into(),
            tabs: value.tabs.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<&ProjectLayout> for StrictProjectLayout {
    fn from(value: &ProjectLayout) -> Self {
        Self {
            project: (&value.project).into(),
            tabs: value.tabs.iter().map(Into::into).collect(),
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct StrictProjectConfig {
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_tab: Option<String>,
}

impl From<StrictProjectConfig> for ProjectConfig {
    fn from(value: StrictProjectConfig) -> Self {
        Self {
            name: value.name,
            default_tab: value.default_tab,
        }
    }
}

impl From<&ProjectConfig> for StrictProjectConfig {
    fn from(value: &ProjectConfig) -> Self {
        Self {
            name: value.name.clone(),
            default_tab: value.default_tab.clone(),
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct StrictTabConfig {
    id: String,
    title: String,
    layout: StrictLayoutNode,
}

impl From<StrictTabConfig> for TabConfig {
    fn from(value: StrictTabConfig) -> Self {
        Self {
            id: value.id,
            title: value.title,
            layout: value.layout.into(),
        }
    }
}

impl From<&TabConfig> for StrictTabConfig {
    fn from(value: &TabConfig) -> Self {
        Self {
            id: value.id.clone(),
            title: value.title.clone(),
            layout: (&value.layout).into(),
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StrictLayoutNode {
    Pane(StrictPaneConfig),
    Split(StrictSplitConfig),
}

impl From<StrictLayoutNode> for LayoutNode {
    fn from(value: StrictLayoutNode) -> Self {
        match value {
            StrictLayoutNode::Pane(pane) => Self::Pane(pane.into()),
            StrictLayoutNode::Split(split) => Self::Split(split.into()),
        }
    }
}

impl From<&LayoutNode> for StrictLayoutNode {
    fn from(value: &LayoutNode) -> Self {
        match value {
            LayoutNode::Pane(pane) => Self::Pane(pane.into()),
            LayoutNode::Split(split) => Self::Split(split.into()),
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct StrictPaneConfig {
    id: String,
    title: String,
    command: String,
    #[serde(default, skip_serializing_if = "is_shell_kind")]
    kind: PaneKind,
    #[serde(default, skip_serializing_if = "is_false")]
    notify_on_exit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    detector: Option<String>,
}

impl From<StrictPaneConfig> for PaneConfig {
    fn from(value: StrictPaneConfig) -> Self {
        Self {
            id: value.id,
            title: value.title,
            command: value.command,
            kind: value.kind,
            notify_on_exit: value.notify_on_exit,
            detector: value.detector,
        }
    }
}

impl From<&PaneConfig> for StrictPaneConfig {
    fn from(value: &PaneConfig) -> Self {
        Self {
            id: value.id.clone(),
            title: value.title.clone(),
            command: value.command.clone(),
            kind: value.kind.clone(),
            notify_on_exit: value.notify_on_exit,
            detector: value.detector.clone(),
        }
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct StrictSplitConfig {
    direction: SplitDirection,
    ratio: f32,
    left: Box<StrictLayoutNode>,
    right: Box<StrictLayoutNode>,
}

impl From<StrictSplitConfig> for SplitConfig {
    fn from(value: StrictSplitConfig) -> Self {
        Self {
            direction: value.direction,
            ratio: value.ratio,
            left: Box::new((*value.left).into()),
            right: Box::new((*value.right).into()),
        }
    }
}

impl From<&SplitConfig> for StrictSplitConfig {
    fn from(value: &SplitConfig) -> Self {
        Self {
            direction: value.direction,
            ratio: value.ratio,
            left: Box::new(value.left.as_ref().into()),
            right: Box::new(value.right.as_ref().into()),
        }
    }
}

fn is_shell_kind(kind: &PaneKind) -> bool {
    kind == &PaneKind::Shell
}

fn is_false(value: &bool) -> bool {
    !*value
}
